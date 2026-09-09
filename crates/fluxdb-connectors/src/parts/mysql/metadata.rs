fn mysql_qualified_table(database: &str, table: &str) -> String {
    format!(
        "{}.{}",
        mysql_quote_identifier(database),
        mysql_quote_identifier(table)
    )
}

fn mysql_indexes(config: &ConnectionConfig, path: &ObjectPath) -> fluxdb_core::Result<Vec<IndexInfo>> {
    let database = mysql_database_for_path(config, path)?;
    let options = mysql_connection_url(config)?
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = options.connect().await.map_err(mysql_error)?;
        let rows = sqlx::query(mysql_indexes_query())
            .bind(&database)
            .bind(&path.name)
            .fetch_all(&mut connection)
            .await
            .map_err(mysql_error)?;
        connection.close().await.map_err(mysql_error)?;

        let mut indexes: BTreeMap<String, IndexInfo> = BTreeMap::new();
        for row in rows {
            let name: String = row.try_get("index_name").map_err(mysql_error)?;
            let column: String = row.try_get("column_name").map_err(mysql_error)?;
            let non_unique: i64 = row.try_get("non_unique").map_err(mysql_error)?;
            let index_type: String = row.try_get("index_type").map_err(mysql_error)?;
            let comment: String = row.try_get("index_comment").map_err(mysql_error)?;
            let entry = indexes.entry(name.clone()).or_insert_with(|| IndexInfo {
                is_primary: name == "PRIMARY",
                name,
                columns: Vec::new(),
                is_unique: non_unique == 0,
                index_type: Some(index_type).filter(|value| !value.is_empty()),
                comment: Some(comment).filter(|value| !value.is_empty()),
            });
            entry.columns.push(column);
        }

        Ok(indexes.into_values().collect())
    })
}

fn mysql_indexes_query() -> &'static str {
    r#"
    SELECT
        CAST(index_name AS CHAR) AS index_name,
        CAST(column_name AS CHAR) AS column_name,
        CAST(non_unique AS SIGNED) AS non_unique,
        CAST(index_type AS CHAR) AS index_type,
        CAST(index_comment AS CHAR) AS index_comment
    FROM information_schema.statistics
    WHERE table_schema = ? AND table_name = ?
    ORDER BY index_name, seq_in_index
    "#
}

fn mysql_foreign_keys(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
    mysql_foreign_keys_with_cancel(config, path, &|| false)
}

fn mysql_foreign_keys_with_cancel(
    config: &ConnectionConfig,
    path: &ObjectPath,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
    let database = mysql_database_for_path(config, path)?;
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
                CAST(constraint_name AS CHAR) AS constraint_name,
                CAST(column_name AS CHAR) AS column_name,
                CAST(referenced_table_schema AS CHAR) AS referenced_table_schema,
                CAST(referenced_table_name AS CHAR) AS referenced_table_name,
                CAST(referenced_column_name AS CHAR) AS referenced_column_name
            FROM information_schema.key_column_usage
            WHERE table_schema = ?
              AND table_name = ?
              AND referenced_table_name IS NOT NULL
            ORDER BY constraint_name, ordinal_position
            "#,
        )
        .bind(&database)
        .bind(&path.name)
        .fetch_all(&mut connection), should_cancel)
        .await
        .map_err(mysql_error)?;
        let Some(rows) = rows else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "foreign_keys", table = %path.name, "MySQL metadata query cancelled");
            drop(connection);
            return Ok(Vec::new());
        };
        connection.close().await.map_err(mysql_error)?;

        rows.into_iter()
            .map(|row| {
                Ok(ForeignKeyInfo {
                    name: row.try_get("constraint_name").map_err(mysql_error)?,
                    column: row.try_get("column_name").map_err(mysql_error)?,
                    ref_schema: Some(
                        row.try_get::<String, _>("referenced_table_schema")
                            .map_err(mysql_error)?,
                    )
                    .filter(|value| !value.is_empty()),
                    ref_table: row.try_get("referenced_table_name").map_err(mysql_error)?,
                    ref_column: row.try_get("referenced_column_name").map_err(mysql_error)?,
                })
            })
            .collect()
    })
}

fn mysql_triggers(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<TriggerInfo>> {
    let database = mysql_database_for_path(config, path)?;
    let options = mysql_connection_url(config)?
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = options.connect().await.map_err(mysql_error)?;
        let rows = sqlx::query(
            r#"
            SELECT
                CAST(trigger_name AS CHAR) AS trigger_name,
                CAST(event_manipulation AS CHAR) AS event_manipulation,
                CAST(action_timing AS CHAR) AS action_timing,
                CAST(action_statement AS CHAR) AS action_statement
            FROM information_schema.triggers
            WHERE trigger_schema = ? AND event_object_table = ?
            ORDER BY trigger_name
            "#,
        )
        .bind(&database)
        .bind(&path.name)
        .fetch_all(&mut connection)
        .await
        .map_err(mysql_error)?;
        connection.close().await.map_err(mysql_error)?;

        rows.into_iter()
            .map(|row| {
                Ok(TriggerInfo {
                    name: row.try_get("trigger_name").map_err(mysql_error)?,
                    event: row.try_get("event_manipulation").map_err(mysql_error)?,
                    timing: row.try_get("action_timing").map_err(mysql_error)?,
                    body: Some(row.try_get("action_statement").map_err(mysql_error)?),
                })
            })
            .collect()
    })
}

fn mysql_table_ddl(config: &ConnectionConfig, path: &ObjectPath) -> fluxdb_core::Result<String> {
    let database = mysql_database_for_path(config, path)?;
    let sql = format!(
        "SHOW CREATE TABLE {}.{}",
        mysql_quote_identifier(&database),
        mysql_quote_identifier(&path.name)
    );
    let options = mysql_connection_url(config)?
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = options.connect().await.map_err(mysql_error)?;
        let row = sqlx::query(&sql)
            .fetch_one(&mut connection)
            .await
            .map_err(mysql_error)?;
        connection.close().await.map_err(mysql_error)?;
        row.try_get::<String, _>(1).map_err(mysql_error)
    })
}

fn mysql_database_for_path(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<String> {
    path.database
        .clone()
        .or_else(|| match &config.endpoint {
            Endpoint::Tcp { database, .. } => database.clone(),
            _ => None,
        })
        .filter(|database| !database.is_empty())
        .ok_or_else(|| Error::new(ErrorKind::Connection, "缺少数据库上下文"))
}
