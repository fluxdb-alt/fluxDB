/// 数据页投影会把部分类型 CAST 成 CHAR 并沿用原列名作为输出别名；MySQL 的 ORDER BY
/// 会优先按 SELECT 输出别名排序，整数会退化为字典序。排序必须通过源表别名引用真实列。
const MYSQL_DATA_SOURCE_ALIAS: &str = "__fluxdb_source";

fn mysql_load_data(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    filters: &[FilterSpec],
) -> fluxdb_core::Result<DataPage> {
    mysql_load_data_with_expression(config, path, offset, limit, sort, filters, None)
}

fn mysql_load_data_expression(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    expression: &fluxdb_core::FilterExpression,
) -> fluxdb_core::Result<DataPage> {
    mysql_load_data_with_expression(config, path, offset, limit, sort, &[], Some(expression))
}

fn mysql_load_data_with_expression(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    filters: &[FilterSpec],
    expression: Option<&fluxdb_core::FilterExpression>,
) -> fluxdb_core::Result<DataPage> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(ErrorKind::Unsupported, "仅表和视图支持数据读取"));
    }

    let database = path
        .database
        .as_deref()
        .ok_or_else(|| Error::new(ErrorKind::Query, "缺少数据库名称"))?;
    let pagination = Pagination::new(offset, limit);
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = plan.connect().await?;

        let columns = mysql_columns(&mut connection, database, &path.name).await?;
        mysql_ensure_columns_available(database, &path.name, &columns)?;
        let select_list = columns
            .iter()
            .map(mysql_select_expr)
            .collect::<Vec<_>>()
            .join(", ");
        let mut builder = QueryBuilder::<MySql>::new(format!(
            "SELECT {select_list} FROM {}.{} AS {}",
            fluxdb_core::MySqlDialect.quote_identifier(database),
            fluxdb_core::MySqlDialect.quote_identifier(&path.name),
            fluxdb_core::MySqlDialect.quote_identifier(MYSQL_DATA_SOURCE_ALIAS)
        ));
        if let Some(expression) = expression {
            if let Some(tree) = crate::filter_dialects::compile_filter_tree(
                DatabaseKind::MySql,
                expression,
                &columns,
            )
            .map_err(|error| Error::new(ErrorKind::Query, error.to_string()))?
            {
                push_expression_where_clause(&mut builder, &tree, push_mysql_filter_bind);
            }
        } else {
            push_data_where_clause(
                &mut builder,
                filters,
                &columns,
                |value| fluxdb_core::MySqlDialect.quote_identifier(value),
                push_mysql_bind,
            );
        }
        builder.push(mysql_order_by_clause(sort, &columns));
        builder.push(" LIMIT ");
        builder.push_bind(pagination.limit + 1);
        builder.push(" OFFSET ");
        builder.push_bind(pagination.offset);

        // 流式取行：大字段行一读进来就外置，整页超出预算就停读，
        // 不会像 `fetch_all` 那样先把整个结果集堆进内存。
        let stream = builder.build().fetch(&mut connection);
        let owner = fluxdb_core::table_content_owner(path);
        let (page, _read_rows) = collect_sqlx_page(
            stream,
            columns,
            pagination,
            0,
            &owner,
            |row, _index, column| mysql_cell_value(row, column),
            mysql_error,
        )
        .await?;
        connection.close().await.map_err(mysql_error)?;
        Ok(page)
    })
}

/// 用户排序后面追加主键作为稳定 tie breaker（设计 7.2），避免同值行分页随并发漂移。
/// 主键列若已在用户排序中出现则跳过；无主键时保持共享排序原样。
/// 排序列统一用源表别名限定，确保按真实列而非 SELECT 输出别名排序。
fn mysql_order_by_clause(sort: &[SortSpec], columns: &[Column]) -> String {
    let user_clause = data_order_by_clause(sort, columns, mysql_data_source_column);
    let sorted: Vec<&str> = sort.iter().map(|spec| spec.field.as_str()).collect();
    let tie: Vec<String> = columns
        .iter()
        .filter(|column| column.primary_key && !sorted.contains(&column.name.as_str()))
        .map(|column| mysql_data_source_column(&column.name))
        .collect();
    if tie.is_empty() {
        return user_clause;
    }
    if user_clause.is_empty() {
        format!(" ORDER BY {}", tie.join(", "))
    } else {
        format!("{user_clause}, {}", tie.join(", "))
    }
}

fn mysql_data_source_column(column: &str) -> String {
    format!(
        "{}.{}",
        fluxdb_core::MySqlDialect.quote_identifier(MYSQL_DATA_SOURCE_ALIAS),
        fluxdb_core::MySqlDialect.quote_identifier(column)
    )
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
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = plan.connect().await?;

        let columns = mysql_columns(&mut connection, database, &path.name).await?;
        let table_name = format!(
            "{}.{}",
            fluxdb_core::MySqlDialect.quote_identifier(database),
            fluxdb_core::MySqlDialect.quote_identifier(&path.name)
        );
        let mut builder =
            QueryBuilder::<MySql>::new(format!("SELECT COUNT(*) AS row_count FROM {table_name}"));
        push_data_where_clause(
            &mut builder,
            filters,
            &columns,
            |value| fluxdb_core::MySqlDialect.quote_identifier(value),
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
                |value| fluxdb_core::MySqlDialect.quote_identifier(value),
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
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = plan.connect().await?;

        let columns = mysql_columns(&mut connection, database, &path.name).await?;
        ensure_column_exists(column, &columns)?;
        let table_name = format!(
            "{}.{}",
            fluxdb_core::MySqlDialect.quote_identifier(database),
            fluxdb_core::MySqlDialect.quote_identifier(&path.name)
        );
        let mut builder = QueryBuilder::<MySql>::new(format!(
            "SELECT {} FROM {table_name}",
            fluxdb_core::MySqlDialect.quote_identifier(column)
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

fn push_mysql_filter_bind(
    builder: &mut QueryBuilder<'_, MySql>,
    value: &fluxdb_core::FilterValue,
) {
    use fluxdb_core::FilterValue;
    match value {
        FilterValue::Null => {
            builder.push_bind(Option::<String>::None);
        }
        FilterValue::Text(value) => {
            builder.push_bind(value.clone());
        }
        FilterValue::Integer(value) => {
            builder.push_bind(*value);
        }
        FilterValue::Float(value) => {
            builder.push_bind(*value);
        }
        FilterValue::Boolean(value) => {
            builder.push_bind(*value);
        }
    }
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
