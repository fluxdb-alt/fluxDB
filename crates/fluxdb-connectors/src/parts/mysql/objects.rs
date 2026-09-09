fn mysql_list_objects(
    config: &ConnectionConfig,
    path: Option<&ObjectPath>,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    if !is_mysql_protocol_kind(config.kind) {
        return Err(Error::new(ErrorKind::Connection, "连接类型不匹配"));
    }

    let url = mysql_connection_url(config)?;
    let options = url
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
    let configured_database = match &config.endpoint {
        Endpoint::Tcp { database, .. } => {
            database.as_deref().filter(|database| !database.is_empty())
        }
        _ => None,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let connect = tokio::time::timeout(Duration::from_secs(5), options.connect()).await;
        let mut connection = match connect {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => return Err(mysql_error(error)),
            Err(_) => return Err(Error::new(ErrorKind::Connection, "连接超时")),
        };

        let objects = if let Some(path) = path {
            let database = path
                .database
                .as_deref()
                .filter(|database| !database.is_empty())
                .unwrap_or(&path.name);
            mysql_table_objects(config.id, &mut connection, database).await?
        } else if let Some(database) = configured_database {
            vec![database_object(config.id, database)]
        } else {
            mysql_database_objects(config.id, &mut connection).await?
        };

        connection.close().await.map_err(mysql_error)?;
        Ok(objects)
    })
}

async fn mysql_database_objects(
    connection_id: ConnectionId,
    connection: &mut sqlx::MySqlConnection,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let rows = sqlx::query(
        r#"
        SELECT CAST(schema_name AS CHAR) AS database_name
        FROM information_schema.schemata
        WHERE schema_name NOT IN ('information_schema', 'mysql', 'performance_schema', 'sys')
        ORDER BY schema_name
        "#,
    )
    .fetch_all(connection)
    .await
    .map_err(mysql_error)?;

    let mut objects = Vec::with_capacity(rows.len());
    for row in rows {
        let database: String = row.try_get("database_name").map_err(mysql_error)?;
        objects.push(database_object(connection_id, &database));
    }
    Ok(objects)
}

async fn mysql_table_objects(
    connection_id: ConnectionId,
    connection: &mut sqlx::MySqlConnection,
    database: &str,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let rows = sqlx::query(mysql_table_objects_query())
    .bind(database)
    .fetch_all(&mut *connection)
    .await
    .map_err(mysql_error)?;

    let mut objects = Vec::with_capacity(rows.len());
    for row in rows {
        let database: String = row.try_get("database_name").map_err(mysql_error)?;
        let name: String = row.try_get("object_name").map_err(mysql_error)?;
        let table_type: String = row.try_get("object_type").map_err(mysql_error)?;
        let rows = mysql_object_row_count(row.try_get("row_count").map_err(mysql_error)?);
        let modified_at: Option<String> = row.try_get("modified_at").map_err(mysql_error)?;
        let comment: Option<String> = row.try_get("object_comment").map_err(mysql_error)?;
        let kind = if table_type.eq_ignore_ascii_case("VIEW") {
            ObjectKind::View
        } else {
            ObjectKind::Table
        };

        objects.push(ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some(database),
                schema: None,
                name,
                kind,
            },
            rows,
            modified_at: modified_at.filter(|value| !value.is_empty()),
            comment: comment.filter(|comment| !comment.is_empty()),
        });
    }
    if objects.is_empty() {
        objects = mysql_show_table_objects(connection_id, connection, database).await?;
    }
    Ok(objects)
}

async fn mysql_show_table_objects(
    connection_id: ConnectionId,
    connection: &mut sqlx::MySqlConnection,
    database: &str,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let rows = sqlx::query(&mysql_show_table_objects_query(database))
    .fetch_all(connection)
    .await
    .map_err(mysql_error)?;

    let mut objects = Vec::with_capacity(rows.len());
    for row in rows {
        let name: String = row.try_get(0).map_err(mysql_error)?;
        let table_type: String = row.try_get(1).map_err(mysql_error)?;
        let kind = if table_type.eq_ignore_ascii_case("VIEW") {
            ObjectKind::View
        } else {
            ObjectKind::Table
        };
        objects.push(ObjectSummary {
            path: ObjectPath {
                connection_id,
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
    Ok(objects)
}

fn mysql_show_table_objects_query(database: &str) -> String {
    format!("SHOW FULL TABLES FROM {}", mysql_quote_identifier(database))
}

fn mysql_table_objects_query() -> &'static str {
    r#"
    SELECT
        CAST(table_schema AS CHAR) AS database_name,
        CAST(table_name AS CHAR) AS object_name,
        CAST(table_type AS CHAR) AS object_type,
        CAST(table_rows AS SIGNED) AS row_count,
        CAST(update_time AS CHAR) AS modified_at,
        CAST(table_comment AS CHAR) AS object_comment
    FROM information_schema.tables
    WHERE table_schema = ?
    ORDER BY table_schema, table_name
    "#
}

fn mysql_object_row_count(value: Option<i64>) -> Option<u64> {
    value.and_then(|value| u64::try_from(value).ok())
}
