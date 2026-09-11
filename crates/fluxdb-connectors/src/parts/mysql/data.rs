fn mysql_load_data(
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

    let database = path
        .database
        .as_deref()
        .ok_or_else(|| Error::new(ErrorKind::Query, "缺少数据库名称"))?;
    let pagination = Pagination::new(offset, limit);
    let url = mysql_connection_url(config)?;
    let options = url
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
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

        let columns = mysql_columns(&mut connection, database, &path.name).await?;
        mysql_ensure_columns_available(database, &path.name, &columns)?;
        let select_list = columns
            .iter()
            .map(mysql_select_expr)
            .collect::<Vec<_>>()
            .join(", ");
        let mut builder = QueryBuilder::<MySql>::new(format!(
            "SELECT {select_list} FROM {}.{}",
            mysql_quote_identifier(database),
            mysql_quote_identifier(&path.name)
        ));
        push_data_where_clause(
            &mut builder,
            filters,
            &columns,
            mysql_quote_identifier,
            push_mysql_bind,
        );
        builder.push(data_order_by_clause(sort, &columns, mysql_quote_identifier));
        builder.push(" LIMIT ");
        builder.push_bind(pagination.limit + 1);
        builder.push(" OFFSET ");
        builder.push_bind(pagination.offset);

        let rows = builder
            .build()
            .fetch_all(&mut connection)
            .await
            .map_err(mysql_error)?;
        connection.close().await.map_err(mysql_error)?;

        Ok(mysql_rows_to_page(columns, rows, pagination))
    })
}

fn mysql_preview_data_export(
    config: &ConnectionConfig,
    path: &ObjectPath,
    fields: &[String],
    sort: &[SortSpec],
    filters: &[FilterSpec],
) -> fluxdb_core::Result<DataExportPreview> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(ErrorKind::Unsupported, "仅表和视图支持导出预览"));
    }

    let database = path
        .database
        .as_deref()
        .ok_or_else(|| Error::new(ErrorKind::Query, "缺少数据库名称"))?;
    let url = mysql_connection_url(config)?;
    let options = url
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
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

        let columns = mysql_columns(&mut connection, database, &path.name).await?;
        let table_name = format!(
            "{}.{}",
            mysql_quote_identifier(database),
            mysql_quote_identifier(&path.name)
        );
        let mut builder =
            QueryBuilder::<MySql>::new(format!("SELECT COUNT(*) AS row_count FROM {table_name}"));
        push_data_where_clause(
            &mut builder,
            filters,
            &columns,
            mysql_quote_identifier,
            push_mysql_bind,
        );
        let row = builder
            .build()
            .fetch_one(&mut connection)
            .await
            .map_err(mysql_error)?;
        connection.close().await.map_err(mysql_error)?;
        let count = row.try_get::<i64, _>("row_count").map_err(mysql_error)?;

        Ok(DataExportPreview {
            sql: data_export_preview_sql(
                &table_name,
                fields,
                sort,
                filters,
                &columns,
                mysql_quote_identifier,
            ),
            row_count: count.max(0) as u64,
        })
    })
}

fn mysql_load_cell_binary(
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

    let database = path
        .database
        .as_deref()
        .ok_or_else(|| Error::new(ErrorKind::Query, "缺少数据库名称"))?;
    let url = mysql_connection_url(config)?;
    let options = url
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
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

        let columns = mysql_columns(&mut connection, database, &path.name).await?;
        ensure_column_exists(column, &columns)?;
        let table_name = format!(
            "{}.{}",
            mysql_quote_identifier(database),
            mysql_quote_identifier(&path.name)
        );
        let mut builder = QueryBuilder::<MySql>::new(format!(
            "SELECT {} FROM {table_name}",
            mysql_quote_identifier(column)
        ));
        push_mysql_identity_where(&mut builder, identity, &columns)?;
        builder.push(" LIMIT 1");
        let row = builder
            .build()
            .fetch_one(&mut connection)
            .await
            .map_err(mysql_error)?;
        connection.close().await.map_err(mysql_error)?;
        row.try_get::<Option<Vec<u8>>, _>(0)
            .map_err(mysql_error)?
            .ok_or_else(|| Error::new(ErrorKind::Query, "二进制单元格为 NULL"))
    })
}

/// 校验字段信息是否足以拼出 SELECT 列表。
///
/// `information_schema.columns` 会按当前账号权限过滤，缺权限时返回空列表；此时 `select_list`
/// 为空会拼出 `SELECT  FROM \`db\`.\`table\``，服务端只回一句 1064 语法错误——既看不出真实原因，
/// 也容易被误判成 SQL 生成 bug。这里提前收敛成可读错误。
fn mysql_ensure_columns_available(
    database: &str,
    table: &str,
    columns: &[Column],
) -> fluxdb_core::Result<()> {
    if !columns.is_empty() {
        return Ok(());
    }

    tracing::warn!(
        target: "fluxdb_connectors",
        database,
        table,
        "MySQL 未读取到字段信息，跳过数据查询"
    );
    Err(Error::new(
        ErrorKind::Query,
        format!("未读取到 `{database}`.`{table}` 的字段信息，请确认当前账号是否有该表的权限"),
    ))
}

async fn mysql_columns(
    connection: &mut sqlx::MySqlConnection,
    database: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<Column>> {
    let rows = sqlx::query(
        r#"
        SELECT
            CAST(column_name AS CHAR) AS column_name,
            CAST(column_type AS CHAR) AS column_type,
            CAST(is_nullable AS CHAR) AS is_nullable,
            CAST(column_key AS CHAR) AS column_key,
            CAST(column_comment AS CHAR) AS column_comment
        FROM information_schema.columns
        WHERE table_schema = ? AND table_name = ?
        ORDER BY ordinal_position
        "#,
    )
    .bind(database)
    .bind(table)
    .fetch_all(connection)
    .await
    .map_err(mysql_error)?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let name: String = row.try_get("column_name").map_err(mysql_error)?;
            let type_name: String = row.try_get("column_type").map_err(mysql_error)?;
            let nullable: String = row.try_get("is_nullable").map_err(mysql_error)?;
            let key: String = row.try_get("column_key").map_err(mysql_error)?;
            let comment: String = row.try_get("column_comment").map_err(mysql_error)?;
            Ok(Column {
                name,
                type_name: Some(type_name),
                nullable: nullable.eq_ignore_ascii_case("YES"),
                primary_key: key.eq_ignore_ascii_case("PRI"),
                comment: Some(comment).filter(|value| !value.is_empty()),
            })
        })
        .collect::<fluxdb_core::Result<Vec<_>>>()?)
}
