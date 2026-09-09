fn mysql_completion_tables(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionTable>> {
    mysql_completion_tables_with_cancel(config, database, _schema, filter, limit, &|| false)
}

fn mysql_completion_tables_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    filter: &str,
    limit: u64,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionTable>> {
    let database = completion_database(config, database)?;
    let options = mysql_connection_url(config)?
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let Some(mut connection) = await_with_cancel(options.connect(), should_cancel)
            .await
            .map_err(mysql_error)? else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "connect", "MySQL metadata connection cancelled");
            return Ok(Vec::new());
        };
        let rows = await_with_cancel(sqlx::query(
            r#"
            SELECT
                CAST(table_name AS CHAR) AS table_name,
                CAST(table_type AS CHAR) AS table_type
            FROM information_schema.tables
            WHERE table_schema = ?
              AND LOWER(table_name) LIKE ?
              ESCAPE '\\'
            ORDER BY table_name
            LIMIT ?
            "#,
        )
        .bind(&database)
        .bind(completion_fuzzy_like_filter(filter))
        .bind(limit as i64)
        .fetch_all(&mut connection)
        , should_cancel)
        .await
        .map_err(mysql_error)?;
        let Some(rows) = rows else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "tables", "MySQL metadata query cancelled");
            drop(connection);
            return Ok(Vec::new());
        };
        connection.close().await.map_err(mysql_error)?;

        rows.into_iter()
            .map(|row| {
                let name: String = row.try_get("table_name").map_err(mysql_error)?;
                let table_type: String = row.try_get("table_type").map_err(mysql_error)?;
                Ok(CompletionTable {
                    database: Some(database.clone()),
                    schema: None,
                    name,
                    kind: if table_type.eq_ignore_ascii_case("VIEW") {
                        ObjectKind::View
                    } else {
                        ObjectKind::Table
                    },
                })
            })
            .collect()
    })
}

fn mysql_completion_columns(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    table: &str,
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    mysql_completion_columns_with_cancel(config, database, _schema, table, &|| false)
}

fn mysql_completion_columns_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    table: &str,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    let database = completion_database(config, database)?;
    let options = mysql_connection_url(config)?
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let Some(mut connection) = await_with_cancel(options.connect(), should_cancel)
            .await
            .map_err(mysql_error)? else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "connect", "MySQL metadata connection cancelled");
            return Ok(Vec::new());
        };
        let columns = await_with_cancel(mysql_columns(&mut connection, &database, table), should_cancel)
            .await?;
        let Some(columns) = columns else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "columns", table = %table, "MySQL metadata query cancelled");
            drop(connection);
            return Ok(Vec::new());
        };
        connection.close().await.map_err(mysql_error)?;
        Ok(columns_to_completion(table, columns))
    })
}

fn mysql_completion_columns_for_tables(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    tables: &[String],
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    mysql_completion_columns_for_tables_with_cancel(config, database, _schema, tables, &|| false)
}

fn mysql_completion_columns_for_tables_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    tables: &[String],
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    if tables.is_empty() {
        return Ok(Vec::new());
    }

    let database = completion_database(config, database)?;
    let options = mysql_connection_url(config)?
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let Some(mut connection) = await_with_cancel(options.connect(), should_cancel)
            .await
            .map_err(mysql_error)? else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "connect", "MySQL metadata connection cancelled");
            return Ok(Vec::new());
        };
        let mut builder = QueryBuilder::<MySql>::new(
            r#"
            SELECT
                CAST(table_name AS CHAR) AS table_name,
                CAST(column_name AS CHAR) AS column_name,
                CAST(column_type AS CHAR) AS column_type,
                CAST(is_nullable AS CHAR) AS is_nullable,
                CAST(column_key AS CHAR) AS column_key,
                CAST(column_comment AS CHAR) AS column_comment
            FROM information_schema.columns
            WHERE table_schema = 
            "#,
        );
        builder.push_bind(&database);
        builder.push(" AND table_name IN (");
        let mut separated = builder.separated(", ");
        for table in tables {
            separated.push_bind(table);
        }
        separated.push_unseparated(") ORDER BY table_name, ordinal_position");

        let rows = await_with_cancel(builder.build().fetch_all(&mut connection), should_cancel)
            .await
            .map_err(mysql_error)?;
        let Some(rows) = rows else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "columns_batch", "MySQL metadata query cancelled");
            drop(connection);
            return Ok(Vec::new());
        };
        connection.close().await.map_err(mysql_error)?;

        rows.into_iter()
            .map(|row| {
                let table: String = row.try_get("table_name").map_err(mysql_error)?;
                let name: String = row.try_get("column_name").map_err(mysql_error)?;
                let column_type: String = row.try_get("column_type").map_err(mysql_error)?;
                let nullable: String = row.try_get("is_nullable").map_err(mysql_error)?;
                let key: String = row.try_get("column_key").map_err(mysql_error)?;
                Ok(CompletionColumn {
                    table,
                    name,
                    type_name: Some(column_type),
                    nullable: nullable.eq_ignore_ascii_case("YES"),
                    primary_key: key.eq_ignore_ascii_case("PRI"),
                    comment: row.try_get("column_comment").ok(),
                })
            })
            .collect()
    })
}

fn mysql_completion_routines(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
    mysql_completion_routines_with_cancel(config, database, _schema, filter, limit, &|| false)
}

fn mysql_completion_routines_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    filter: &str,
    limit: u64,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
    let database = completion_database(config, database)?;
    let options = mysql_connection_url(config)?
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let Some(mut connection) = await_with_cancel(options.connect(), should_cancel)
            .await
            .map_err(mysql_error)? else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "connect", "MySQL metadata connection cancelled");
            return Ok(Vec::new());
        };
        let rows = await_with_cancel(sqlx::query(
            r#"
            SELECT
                CAST(routine_name AS CHAR) AS routine_name,
                CAST(routine_type AS CHAR) AS routine_type
            FROM information_schema.routines
            WHERE routine_schema = ?
              AND LOWER(routine_name) LIKE ?
            ORDER BY routine_name
            LIMIT ?
            "#,
        )
        .bind(&database)
        .bind(completion_like_filter(filter))
        .bind(limit as i64)
        .fetch_all(&mut connection), should_cancel)
        .await;
        let Some(rows) = rows.map_err(mysql_error)? else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "routines", "MySQL metadata query cancelled");
            drop(connection);
            return Ok(Vec::new());
        };
        connection.close().await.map_err(mysql_error)?;
        rows.into_iter()
            .map(|row| {
                let name: String = row.try_get("routine_name").map_err(mysql_error)?;
                let routine_type: String = row.try_get("routine_type").map_err(mysql_error)?;
                Ok(CompletionRoutine {
                    schema: None,
                    name,
                    kind: if routine_type.eq_ignore_ascii_case("PROCEDURE") {
                        CompletionRoutineKind::Procedure
                    } else {
                        CompletionRoutineKind::Function
                    },
                })
            })
            .collect()
    })
}

fn mysql_completion_triggers(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
    mysql_completion_triggers_with_cancel(config, database, _schema, filter, limit, &|| false)
}

fn mysql_completion_triggers_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    _schema: Option<&str>,
    filter: &str,
    limit: u64,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
    let database = completion_database(config, database)?;
    let options = mysql_connection_url(config)?
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let Some(mut connection) = await_with_cancel(options.connect(), should_cancel)
            .await
            .map_err(mysql_error)? else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "connect", "MySQL metadata connection cancelled");
            return Ok(Vec::new());
        };
        let rows = await_with_cancel(sqlx::query(
            r#"
            SELECT
                CAST(trigger_name AS CHAR) AS trigger_name,
                CAST(event_object_table AS CHAR) AS table_name
            FROM information_schema.triggers
            WHERE trigger_schema = ?
              AND LOWER(trigger_name) LIKE ?
            ORDER BY trigger_name
            LIMIT ?
            "#,
        )
        .bind(&database)
        .bind(completion_like_filter(filter))
        .bind(limit as i64)
        .fetch_all(&mut connection), should_cancel)
        .await;
        let Some(rows) = rows.map_err(mysql_error)? else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "triggers", "MySQL metadata query cancelled");
            drop(connection);
            return Ok(Vec::new());
        };
        connection.close().await.map_err(mysql_error)?;
        rows.into_iter()
            .map(|row| {
                Ok(CompletionTrigger {
                    schema: None,
                    name: row.try_get("trigger_name").map_err(mysql_error)?,
                    table: row.try_get("table_name").ok(),
                })
            })
            .collect()
    })
}
