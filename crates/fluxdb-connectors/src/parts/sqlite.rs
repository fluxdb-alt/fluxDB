fn sqlite_connection_options(config: &ConnectionConfig) -> fluxdb_core::Result<SqliteConnectOptions> {
    let Endpoint::SqliteFile { path, read_only } = &config.endpoint else {
        return Err(Error::new(
            ErrorKind::Connection,
            "SQLite 连接需要数据库文件路径",
        ));
    };

    if path.as_os_str().is_empty() {
        return Err(Error::new(ErrorKind::Connection, "SQLite 文件路径不能为空"));
    }

    if path == Path::new(":memory:") {
        return Ok(SqliteConnectOptions::new().in_memory(true));
    }

    if *read_only && !path.exists() {
        return Err(Error::new(
            ErrorKind::Connection,
            "SQLite 只读连接要求文件已存在",
        ));
    }

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        return Err(Error::new(
            ErrorKind::Connection,
            format!("SQLite 文件目录不存在：{}", parent.display()),
        ));
    }

    Ok(SqliteConnectOptions::new()
        .filename(path)
        .read_only(*read_only)
        .create_if_missing(!*read_only))
}

async fn sqlite_connect(config: &ConnectionConfig) -> fluxdb_core::Result<sqlx::SqliteConnection> {
    let options = sqlite_connection_options(config)?;
    let mut connection = options.connect().await.map_err(sqlite_error)?;
    for (database, path) in sqlite_attached_databases(config) {
        if !path.exists() {
            return Err(Error::new(
                ErrorKind::Connection,
                format!("SQLite 附加数据库不存在：{}", path.display()),
            ));
        }
        sqlx::query(&format!(
            "ATTACH DATABASE ? AS {}",
            sqlite_quote_identifier(&database)
        ))
        .bind(path.to_string_lossy().to_string())
        .execute(&mut connection)
        .await
        .map_err(sqlite_error)?;
    }
    Ok(connection)
}

fn sqlite_database_for_path(path: &ObjectPath) -> &str {
    path.database.as_deref().unwrap_or("main")
}

fn sqlite_qualified_table(database: &str, table: &str) -> String {
    format!(
        "{}.{}",
        sqlite_quote_identifier(database),
        sqlite_quote_identifier(table)
    )
}

fn sqlite_list_objects(
    config: &ConnectionConfig,
    path: Option<&ObjectPath>,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    if config.kind != DatabaseKind::Sqlite {
        return Err(Error::new(ErrorKind::Connection, "连接类型不匹配"));
    }

    if path.is_none() {
        let mut objects = vec![database_object(config.id, "main")];
        objects.extend(
            sqlite_attached_databases(config)
                .into_iter()
                .map(|(database, _)| database_object(config.id, &database)),
        );
        return Ok(objects);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let connect = tokio::time::timeout(Duration::from_secs(5), sqlite_connect(config)).await;
        let mut connection = match connect {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(Error::new(ErrorKind::Connection, "连接超时")),
        };

        let database = path.map(sqlite_database_for_path).unwrap_or("main");
        let rows = sqlx::query(&format!(
            r#"
            SELECT name, type
            FROM {}.sqlite_schema
            WHERE type IN ('table', 'view')
              AND name NOT LIKE 'sqlite_%'
            ORDER BY type, name
            "#,
            sqlite_quote_identifier(database)
        ))
        .fetch_all(&mut connection)
        .await
        .map_err(sqlite_error)?;

        let mut objects = Vec::with_capacity(rows.len());
        for row in rows {
            let name: String = row.try_get("name").map_err(sqlite_error)?;
            let object_type: String = row.try_get("type").map_err(sqlite_error)?;
            let kind = if object_type == "view" {
                ObjectKind::View
            } else {
                ObjectKind::Table
            };

            objects.push(ObjectSummary {
                path: ObjectPath {
                    connection_id: config.id,
                    database: Some(database.to_string()),
                    schema: None,
                    name,
                    kind,
                },
                rows: None,
                modified_at: None,
                comment: None,
            });
        }

        connection.close().await.map_err(sqlite_error)?;
        Ok(objects)
    })
}

fn sqlite_create_database(
    config: &ConnectionConfig,
    request: &CreateDatabaseRequest,
) -> fluxdb_core::Result<()> {
    if config.kind != DatabaseKind::Sqlite {
        return Err(Error::new(ErrorKind::Connection, "连接类型不支持新建数据库"));
    }
    if config.id != request.connection_id {
        return Err(Error::new(ErrorKind::Connection, "连接不匹配"));
    }
    if request.name.trim().is_empty() {
        return Err(Error::new(ErrorKind::Query, "数据库名称不能为空"));
    }
    if matches!(&config.endpoint, Endpoint::SqliteFile { read_only: true, .. }) {
        return Err(Error::new(ErrorKind::Permission, "只读 SQLite 连接不支持新建数据库"));
    }

    let path = request
        .path
        .as_ref()
        .ok_or_else(|| Error::new(ErrorKind::Connection, "SQLite 新建数据库需要文件路径"))?;
    if path.as_os_str().is_empty() || path == Path::new(":memory:") {
        return Err(Error::new(ErrorKind::Connection, "SQLite 文件路径不能为空"));
    }
    if path.exists() {
        return Err(Error::new(
            ErrorKind::Connection,
            format!("SQLite 文件已存在：{}", path.display()),
        ));
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        return Err(Error::new(
            ErrorKind::Connection,
            format!("SQLite 文件目录不存在：{}", parent.display()),
        ));
    }

    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let connect = tokio::time::timeout(Duration::from_secs(5), options.connect()).await;
        let connection = match connect {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => return Err(sqlite_error(error)),
            Err(_) => return Err(Error::new(ErrorKind::Connection, "连接超时")),
        };
        connection.close().await.map_err(sqlite_error)
    })
}

fn sqlite_load_data(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    filters: &[FilterSpec],
) -> fluxdb_core::Result<DataPage> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(ErrorKind::Unsupported, "仅表和视图支持数据读取"));
    }

    let pagination = Pagination::new(offset, limit);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let connect = tokio::time::timeout(Duration::from_secs(5), sqlite_connect(config)).await;
        let mut connection = match connect {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(Error::new(ErrorKind::Connection, "连接超时")),
        };

        let database = sqlite_database_for_path(path);
        let columns = sqlite_columns(&mut connection, database, &path.name).await?;
        let select_list = columns
            .iter()
            .map(sqlite_select_expr)
            .collect::<Vec<_>>()
            .join(", ");
        let mut builder = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {select_list} FROM {}",
            sqlite_qualified_table(database, &path.name)
        ));
        push_data_where_clause(
            &mut builder,
            filters,
            &columns,
            sqlite_quote_identifier,
            push_sqlite_bind,
        );
        builder.push(data_order_by_clause(
            sort,
            &columns,
            sqlite_quote_identifier,
        ));
        builder.push(" LIMIT ");
        builder.push_bind((pagination.limit + 1) as i64);
        builder.push(" OFFSET ");
        builder.push_bind(pagination.offset as i64);

        let rows = builder
            .build()
            .fetch_all(&mut connection)
            .await
            .map_err(sqlite_error)?;

        connection.close().await.map_err(sqlite_error)?;
        Ok(sqlite_rows_to_page(columns, rows, pagination))
    })
}

fn sqlite_preview_data_export(
    config: &ConnectionConfig,
    path: &ObjectPath,
    fields: &[String],
    sort: &[SortSpec],
    filters: &[FilterSpec],
) -> fluxdb_core::Result<DataExportPreview> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(ErrorKind::Unsupported, "仅表和视图支持导出预览"));
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let connect = tokio::time::timeout(Duration::from_secs(5), sqlite_connect(config)).await;
        let mut connection = match connect {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(Error::new(ErrorKind::Connection, "连接超时")),
        };

        let database = sqlite_database_for_path(path);
        let columns = sqlite_columns(&mut connection, database, &path.name).await?;
        let table_name = sqlite_qualified_table(database, &path.name);
        let mut builder =
            QueryBuilder::<Sqlite>::new(format!("SELECT COUNT(*) AS row_count FROM {table_name}"));
        push_data_where_clause(
            &mut builder,
            filters,
            &columns,
            sqlite_quote_identifier,
            push_sqlite_bind,
        );
        let row = builder
            .build()
            .fetch_one(&mut connection)
            .await
            .map_err(sqlite_error)?;
        connection.close().await.map_err(sqlite_error)?;
        let count = row.try_get::<i64, _>("row_count").map_err(sqlite_error)?;

        Ok(DataExportPreview {
            sql: data_export_preview_sql(
                &table_name,
                fields,
                sort,
                filters,
                &columns,
                sqlite_quote_identifier,
            ),
            row_count: count.max(0) as u64,
        })
    })
}

fn sqlite_completion_tables(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionTable>> {
    sqlite_completion_tables_with_cancel(config, database, _schema, filter, limit, &|| false)
}

fn sqlite_completion_tables_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    filter: &str,
    limit: u64,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionTable>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let Some(mut connection) = await_with_cancel(sqlite_connect(config), should_cancel).await? else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "connect", "SQLite metadata connection cancelled");
            return Ok(Vec::new());
        };
        let database = database.unwrap_or("main");
        let rows = await_with_cancel(sqlx::query(&format!(
            r#"
            SELECT name, type
            FROM {}.sqlite_schema
            WHERE type IN ('table', 'view')
              AND name NOT LIKE 'sqlite_%'
              AND LOWER(name) LIKE ?
              ESCAPE '\'
            ORDER BY type, name
            LIMIT ?
            "#,
            sqlite_quote_identifier(database)
        ))
        .bind(completion_fuzzy_like_filter(filter))
        .bind(limit as i64)
        .fetch_all(&mut connection)
        , should_cancel)
        .await
        .map_err(sqlite_error)?;
        let Some(rows) = rows else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "tables", "SQLite metadata query cancelled");
            drop(connection);
            return Ok(Vec::new());
        };
        connection.close().await.map_err(sqlite_error)?;

        rows.into_iter()
            .map(|row| {
                let name: String = row.try_get("name").map_err(sqlite_error)?;
                let object_type: String = row.try_get("type").map_err(sqlite_error)?;
                Ok(CompletionTable {
                    database: Some(database.to_string()),
                    schema: None,
                    name,
                    kind: if object_type == "view" {
                        ObjectKind::View
                    } else {
                        ObjectKind::Table
                    },
                })
            })
            .collect()
    })
}

fn sqlite_completion_columns(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    table: &str,
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    sqlite_completion_columns_with_cancel(config, database, _schema, table, &|| false)
}

fn sqlite_completion_columns_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    table: &str,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let Some(mut connection) = await_with_cancel(sqlite_connect(config), should_cancel).await? else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "connect", "SQLite metadata connection cancelled");
            return Ok(Vec::new());
        };
        let columns = await_with_cancel(
            sqlite_columns(&mut connection, database.unwrap_or("main"), table),
            should_cancel,
        )
        .await?;
        let Some(columns) = columns else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "columns", table = %table, "SQLite metadata query cancelled");
            drop(connection);
            return Ok(Vec::new());
        };
        connection.close().await.map_err(sqlite_error)?;
        Ok(columns_to_completion(table, columns))
    })
}

fn sqlite_completion_columns_for_tables(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    tables: &[String],
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    sqlite_completion_columns_for_tables_with_cancel(config, database, _schema, tables, &|| false)
}

fn sqlite_completion_columns_for_tables_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    tables: &[String],
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    if tables.is_empty() {
        return Ok(Vec::new());
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let Some(mut connection) = await_with_cancel(sqlite_connect(config), should_cancel).await? else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "connect", "SQLite metadata connection cancelled");
            return Ok(Vec::new());
        };
        let database = database.unwrap_or("main");
        let mut result = Vec::new();
        for table in tables {
            if should_cancel() {
                drop(connection);
                return Ok(Vec::new());
            }
            let columns = await_with_cancel(sqlite_columns(&mut connection, database, table), should_cancel)
                .await?;
            let Some(columns) = columns else {
                tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "columns_batch", "SQLite metadata query cancelled");
                drop(connection);
                return Ok(Vec::new());
            };
            result.extend(columns_to_completion(table, columns));
        }
        connection.close().await.map_err(sqlite_error)?;
        Ok(result)
    })
}

fn sqlite_completion_triggers(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
    sqlite_completion_triggers_with_cancel(config, database, _schema, filter, limit, &|| false)
}

fn sqlite_completion_triggers_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    filter: &str,
    limit: u64,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let Some(mut connection) = await_with_cancel(sqlite_connect(config), should_cancel).await? else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "connect", "SQLite metadata connection cancelled");
            return Ok(Vec::new());
        };
        let database = database.unwrap_or("main");
        let rows = await_with_cancel(sqlx::query(&format!(
            r#"
            SELECT name, tbl_name
            FROM {}.sqlite_schema
            WHERE type = 'trigger'
              AND LOWER(name) LIKE ?
              ESCAPE '\'
            ORDER BY name
            LIMIT ?
            "#,
            sqlite_quote_identifier(database)
        ))
        .bind(completion_fuzzy_like_filter(filter))
        .bind(limit as i64)
        .fetch_all(&mut connection)
        , should_cancel)
        .await
        .map_err(sqlite_error)?;
        let Some(rows) = rows else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "triggers", "SQLite metadata query cancelled");
            drop(connection);
            return Ok(Vec::new());
        };
        connection.close().await.map_err(sqlite_error)?;

        rows.into_iter()
            .map(|row| {
                Ok(CompletionTrigger {
                    schema: None,
                    name: row.try_get("name").map_err(sqlite_error)?,
                    table: row.try_get("tbl_name").ok(),
                })
            })
            .collect()
    })
}

fn sqlite_load_cell_binary(
    config: &ConnectionConfig,
    path: &ObjectPath,
    identity: &fluxdb_core::RowIdentity,
    column: &str,
) -> fluxdb_core::Result<Vec<u8>> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "仅表和视图支持二进制读取",
        ));
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let connect = tokio::time::timeout(Duration::from_secs(5), sqlite_connect(config)).await;
        let mut connection = match connect {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(Error::new(ErrorKind::Connection, "连接超时")),
        };

        let database = sqlite_database_for_path(path);
        let columns = sqlite_columns(&mut connection, database, &path.name).await?;
        ensure_column_exists(column, &columns)?;
        let mut builder = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {} FROM {}",
            sqlite_quote_identifier(column),
            sqlite_qualified_table(database, &path.name)
        ));
        push_sqlite_identity_where(&mut builder, identity, &columns)?;
        builder.push(" LIMIT 1");
        let row = builder
            .build()
            .fetch_one(&mut connection)
            .await
            .map_err(sqlite_error)?;
        connection.close().await.map_err(sqlite_error)?;
        row.try_get::<Option<Vec<u8>>, _>(0)
            .map_err(sqlite_error)?
            .ok_or_else(|| Error::new(ErrorKind::Query, "二进制单元格为 NULL"))
    })
}

fn sqlite_apply_changes(
    config: &ConnectionConfig,
    changes: &DataChangeSet,
) -> fluxdb_core::Result<()> {
    validate_data_changes(changes)?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = sqlite_connect(config).await?;
        let result = async {
            let database = sqlite_database_for_path(&changes.object);
            let columns = sqlite_columns(&mut connection, database, &changes.object.name).await?;
            sqlx::query("BEGIN IMMEDIATE")
                .execute(&mut connection)
                .await
                .map_err(sqlite_error)?;
            sqlite_apply_deletes(&mut connection, database, changes, &columns).await?;
            sqlite_apply_updates(&mut connection, database, changes, &columns).await?;
            sqlite_apply_inserts(&mut connection, database, changes, &columns).await?;
            sqlx::query("COMMIT")
                .execute(&mut connection)
                .await
                .map_err(sqlite_error)?;
            Ok(())
        }
        .await;

        if result.is_err() {
            let _ = sqlx::query("ROLLBACK").execute(&mut connection).await;
        }
        connection.close().await.map_err(sqlite_error)?;
        result
    })
}

async fn sqlite_apply_inserts(
    connection: &mut sqlx::SqliteConnection,
    database: &str,
    changes: &DataChangeSet,
    columns: &[Column],
) -> fluxdb_core::Result<()> {
    let table_name = sqlite_qualified_table(database, &changes.object.name);
    for row in &changes.inserts {
        let insert_values = non_null_insert_values(row, columns)?;
        if insert_values.is_empty() {
            sqlx::query(&format!("INSERT INTO {table_name} DEFAULT VALUES"))
                .execute(&mut *connection)
                .await
                .map_err(sqlite_error)?;
            continue;
        }

        let mut builder = QueryBuilder::<Sqlite>::new(format!("INSERT INTO {table_name} ("));
        push_insert_column_list(&mut builder, &insert_values, sqlite_quote_identifier);
        builder.push(") VALUES (");
        for (index, (_, value)) in insert_values.iter().enumerate() {
            if index > 0 {
                builder.push(", ");
            }
            push_sqlite_bind(&mut builder, *value);
        }
        builder.push(")");
        builder
            .build()
            .execute(&mut *connection)
            .await
            .map_err(sqlite_error)?;
    }

    Ok(())
}

async fn sqlite_apply_updates(
    connection: &mut sqlx::SqliteConnection,
    database: &str,
    changes: &DataChangeSet,
    columns: &[Column],
) -> fluxdb_core::Result<()> {
    let table_name = sqlite_qualified_table(database, &changes.object.name);
    for update in &changes.updates {
        if update.cells.is_empty() {
            continue;
        }
        validate_identity(&update.identity, columns)?;

        let mut builder = QueryBuilder::<Sqlite>::new(format!("UPDATE {table_name} SET "));
        for (index, cell) in update.cells.iter().enumerate() {
            ensure_column_exists(&cell.column, columns)?;
            if index > 0 {
                builder.push(", ");
            }
            builder
                .push(sqlite_quote_identifier(&cell.column))
                .push(" = ");
            push_sqlite_bind(&mut builder, &cell.value);
        }
        push_sqlite_identity_where(&mut builder, &update.identity, columns)?;
        builder
            .build()
            .execute(&mut *connection)
            .await
            .map_err(sqlite_error)?;
    }

    Ok(())
}

async fn sqlite_apply_deletes(
    connection: &mut sqlx::SqliteConnection,
    database: &str,
    changes: &DataChangeSet,
    columns: &[Column],
) -> fluxdb_core::Result<()> {
    let table_name = sqlite_qualified_table(database, &changes.object.name);
    for identity in &changes.deletes {
        validate_identity(identity, columns)?;

        let mut builder = QueryBuilder::<Sqlite>::new(format!("DELETE FROM {table_name}"));
        push_sqlite_identity_where(&mut builder, identity, columns)?;
        builder
            .build()
            .execute(&mut *connection)
            .await
            .map_err(sqlite_error)?;
    }

    Ok(())
}

async fn sqlite_columns(
    connection: &mut sqlx::SqliteConnection,
    database: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<Column>> {
    let rows = sqlx::query(&format!(
        "PRAGMA {}.table_info({})",
        sqlite_quote_identifier(database),
        sqlite_quote_string(table)
    ))
        .fetch_all(connection)
        .await
        .map_err(sqlite_error)?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let name: String = row.try_get("name").map_err(sqlite_error)?;
            let type_name: String = row.try_get("type").map_err(sqlite_error)?;
            let not_null: i64 = row.try_get("notnull").map_err(sqlite_error)?;
            let pk: i64 = row.try_get("pk").map_err(sqlite_error)?;
            Ok(Column {
                name,
                type_name: Some(type_name).filter(|value| !value.is_empty()),
                nullable: not_null == 0,
                primary_key: pk > 0,
                comment: None,
            })
        })
        .collect::<fluxdb_core::Result<Vec<_>>>()?)
}

fn sqlite_indexes(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<IndexInfo>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = sqlite_connect(config).await?;
        let database = sqlite_database_for_path(path);
        let sql = format!(
            "PRAGMA {}.index_list({})",
            sqlite_quote_identifier(database),
            sqlite_quote_string(&path.name)
        );
        let rows = sqlx::query(&sql)
            .fetch_all(&mut connection)
            .await
            .map_err(sqlite_error)?;
        let mut indexes = Vec::with_capacity(rows.len());
        for row in rows {
            let name: String = row.try_get("name").map_err(sqlite_error)?;
            let unique: i64 = row.try_get("unique").map_err(sqlite_error)?;
            let origin: String = row.try_get("origin").map_err(sqlite_error)?;
            let partial: i64 = row.try_get("partial").map_err(sqlite_error)?;
            let info_sql = format!(
                "PRAGMA {}.index_info({})",
                sqlite_quote_identifier(database),
                sqlite_quote_string(&name)
            );
            let info_rows = sqlx::query(&info_sql)
                .fetch_all(&mut connection)
                .await
                .map_err(sqlite_error)?;
            let columns = info_rows
                .into_iter()
                .map(|row| row.try_get("name").map_err(sqlite_error))
                .collect::<fluxdb_core::Result<Vec<String>>>()?;
            indexes.push(IndexInfo {
                name,
                columns,
                is_unique: unique != 0,
                is_primary: origin == "pk",
                index_type: Some(if partial != 0 { "PARTIAL" } else { &origin }.to_string()),
                comment: None,
            });
        }
        connection.close().await.map_err(sqlite_error)?;
        Ok(indexes)
    })
}

fn sqlite_foreign_keys(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
    sqlite_foreign_keys_with_cancel(config, path, &|| false)
}

fn sqlite_foreign_keys_with_cancel(
    config: &ConnectionConfig,
    path: &ObjectPath,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let Some(mut connection) = await_with_cancel(sqlite_connect(config), should_cancel).await? else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "connect", "SQLite metadata connection cancelled");
            return Ok(Vec::new());
        };
        let sql = format!(
            "PRAGMA {}.foreign_key_list({})",
            sqlite_quote_identifier(sqlite_database_for_path(path)),
            sqlite_quote_string(&path.name)
        );
        let rows = await_with_cancel(sqlx::query(&sql).fetch_all(&mut connection), should_cancel)
            .await
            .map_err(sqlite_error)?;
        let Some(rows) = rows else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "foreign_keys", table = %path.name, "SQLite metadata query cancelled");
            drop(connection);
            return Ok(Vec::new());
        };
        connection.close().await.map_err(sqlite_error)?;

        rows.into_iter()
            .map(|row| {
                let id: i64 = row.try_get("id").map_err(sqlite_error)?;
                Ok(ForeignKeyInfo {
                    name: format!("fk_{id}"),
                    column: row.try_get("from").map_err(sqlite_error)?,
                    ref_schema: None,
                    ref_table: row.try_get("table").map_err(sqlite_error)?,
                    ref_column: row.try_get("to").map_err(sqlite_error)?,
                })
            })
            .collect()
    })
}

fn sqlite_triggers(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<TriggerInfo>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = sqlite_connect(config).await?;
        let rows = sqlx::query(&format!(
            r#"
            SELECT name, sql
            FROM {}.sqlite_schema
            WHERE type = 'trigger' AND tbl_name = ?
            ORDER BY name
            "#,
            sqlite_quote_identifier(sqlite_database_for_path(path))
        ))
        .bind(&path.name)
        .fetch_all(&mut connection)
        .await
        .map_err(sqlite_error)?;
        connection.close().await.map_err(sqlite_error)?;

        rows.into_iter()
            .map(|row| {
                let sql: String = row.try_get("sql").map_err(sqlite_error)?;
                Ok(TriggerInfo {
                    name: row.try_get("name").map_err(sqlite_error)?,
                    event: sqlite_trigger_word(&sql, &["INSERT", "UPDATE", "DELETE"]),
                    timing: sqlite_trigger_word(&sql, &["BEFORE", "AFTER", "INSTEAD OF"]),
                    body: sqlite_trigger_body(&sql),
                })
            })
            .collect()
    })
}

fn sqlite_table_ddl(config: &ConnectionConfig, path: &ObjectPath) -> fluxdb_core::Result<String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = sqlite_connect(config).await?;
        let row: Option<(String,)> = sqlx::query_as(&format!(
            "SELECT sql FROM {}.sqlite_schema WHERE type IN ('table', 'view') AND name = ?",
            sqlite_quote_identifier(sqlite_database_for_path(path))
        ))
        .bind(&path.name)
        .fetch_optional(&mut connection)
        .await
        .map_err(sqlite_error)?;
        connection.close().await.map_err(sqlite_error)?;
        row.map(|row| row.0)
            .ok_or_else(|| Error::new(ErrorKind::Query, "表 DDL 不存在"))
    })
}

fn sqlite_trigger_body(sql: &str) -> Option<String> {
    let upper = sql.to_ascii_uppercase();
    let begin = upper.find("BEGIN")?;
    Some(sql[begin..].trim().trim_end_matches(';').to_string())
}
