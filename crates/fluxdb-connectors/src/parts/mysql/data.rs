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
    mysql_load_data_with_expression(config, path, offset, limit, sort, filters, None, &[])
}

/// 带摘要列的分页读取：摘要列只返回 SHA-256 十六进制文本，避免把大字段整列搬进内存。
fn mysql_load_data_digests(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    filters: &[FilterSpec],
    digest_columns: &[String],
) -> fluxdb_core::Result<DataPage> {
    mysql_load_data_with_expression(
        config,
        path,
        offset,
        limit,
        sort,
        filters,
        None,
        digest_columns,
    )
}

fn mysql_load_data_expression(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    expression: &fluxdb_core::FilterExpression,
) -> fluxdb_core::Result<DataPage> {
    mysql_load_data_with_expression(
        config,
        path,
        offset,
        limit,
        sort,
        &[],
        Some(expression),
        &[],
    )
}

fn mysql_load_data_with_expression(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    filters: &[FilterSpec],
    expression: Option<&fluxdb_core::FilterExpression>,
    digest_columns: &[String],
) -> fluxdb_core::Result<DataPage> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(ErrorKind::Unsupported, "仅表和视图支持数据读取"));
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
        let page = mysql_fetch_page(
            &mut connection,
            config,
            database,
            path,
            offset,
            limit,
            sort,
            filters,
            expression,
            digest_columns,
        )
        .await?;
        connection.close().await.map_err(mysql_error)?;
        Ok(page)
    })
}

/// 在既有连接上读取一页数据。备份快照复用同一连接，避免每页重连。
#[allow(clippy::too_many_arguments)]
async fn mysql_fetch_page(
    connection: &mut sqlx::MySqlConnection,
    config: &ConnectionConfig,
    database: &str,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    filters: &[FilterSpec],
    expression: Option<&fluxdb_core::FilterExpression>,
    digest_columns: &[String],
) -> fluxdb_core::Result<DataPage> {
        let pagination = Pagination::new(offset, limit);
        // 数据同步按块逐页读取，走缓存避免每页都查一次 information_schema。
        let columns = mysql_columns_cached(config, connection, database, &path.name).await?;
        mysql_ensure_columns_available(database, &path.name, &columns)?;
        // 摘要列沿用原列名做别名，行数据的列顺序与普通读取完全一致。
        let select_list = columns
            .iter()
            .map(|column| mysql_select_expr_digest(column, digest_columns))
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
        let stream = builder.build().fetch(&mut *connection);
        let owner = fluxdb_core::table_content_owner(path);
        let digest_columns = digest_columns.to_vec();
        let (page, _read_rows) = collect_sqlx_page(
            stream,
            columns,
            pagination,
            0,
            &owner,
            move |row, _index, column| {
                // 摘要列是十六进制文本，直接按文本解码；其余列保持原投影规则。
                if digest_columns
                    .iter()
                    .any(|digest| digest.eq_ignore_ascii_case(&column.name))
                {
                    return row
                        .try_get::<Option<String>, _>(column.name.as_str())
                        .map(|value| value.map(CellValue::Text).unwrap_or(CellValue::Null))
                        .unwrap_or(CellValue::Null);
                }
                mysql_cell_value(row, column)
            },
            mysql_error,
        )
        .await?;
        Ok(page)
}

/// 按主键键集读取下一页；无主键时返回 `Unsupported`，由调用方退回 OFFSET。
///
/// 键集条件用行构造器 `(pk1, pk2) > (?, ?)`，与 `ORDER BY pk1, pk2` 配合，
/// 避免大表 OFFSET 越翻越慢，也避免页边界随并发写入漂移。
#[allow(clippy::too_many_arguments)]
async fn mysql_fetch_page_keyset(
    connection: &mut sqlx::MySqlConnection,
    config: &ConnectionConfig,
    database: &str,
    path: &ObjectPath,
    after: &[CellValue],
    limit: u64,
) -> fluxdb_core::Result<DataPage> {
    let pagination = Pagination::new(0, limit);
    let columns = mysql_columns_cached(config, connection, database, &path.name).await?;
    mysql_ensure_columns_available(database, &path.name, &columns)?;
    let keys = columns
        .iter()
        .filter(|column| column.primary_key)
        .collect::<Vec<_>>();
    if keys.is_empty() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "表没有主键，无法使用键集分页",
        ));
    }
    if !after.is_empty() && after.len() != keys.len() {
        return Err(Error::new(
            ErrorKind::Query,
            "键集游标与主键列数量不一致",
        ));
    }
    // 二进制主键的单元格只有摘要（无原始字节），绑定会退化成 NULL 并静默截断结果；
    // 整页预览/截断值同理。此时退回 OFFSET，不冒险用错误游标继续。
    if after.iter().any(|value| {
        matches!(
            value,
            CellValue::Null
                | CellValue::BinarySummary(_)
                | CellValue::LargeValue(_)
                | CellValue::TruncatedText { .. }
        )
    }) {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "主键值无法安全绑定，退回偏移分页",
        ));
    }
    let select_list = columns
        .iter()
        .map(mysql_select_expr)
        .collect::<Vec<_>>()
        .join(", ");
    let table = format!(
        "{}.{}",
        fluxdb_core::MySqlDialect.quote_identifier(database),
        fluxdb_core::MySqlDialect.quote_identifier(&path.name)
    );
    let mut builder = QueryBuilder::<MySql>::new(format!(
        "SELECT {select_list} FROM {table} AS {}",
        fluxdb_core::MySqlDialect.quote_identifier(MYSQL_DATA_SOURCE_ALIAS)
    ));
    if !after.is_empty() {
        builder.push(" WHERE (");
        for (index, column) in keys.iter().enumerate() {
            if index > 0 {
                builder.push(", ");
            }
            builder.push(mysql_data_source_column(&column.name));
        }
        builder.push(") > (");
        for (index, value) in after.iter().enumerate() {
            if index > 0 {
                builder.push(", ");
            }
            push_mysql_bind(&mut builder, value);
        }
        builder.push(")");
    }
    builder.push(" ORDER BY ");
    for (index, column) in keys.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push(mysql_data_source_column(&column.name));
    }
    builder.push(" LIMIT ");
    builder.push_bind(pagination.limit + 1);

    let stream = builder.build().fetch(&mut *connection);
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
    Ok(page)
}

/// 摘要列用 `SHA2(..., 256)` 覆盖，其余列保持原投影。
///
/// 同一引擎两端摘要表达式一致，所以可以直接比较；不接受跨数据库类型比对。
fn mysql_select_expr_digest(column: &Column, digest_columns: &[String]) -> String {
    if digest_columns
        .iter()
        .any(|name| name.eq_ignore_ascii_case(&column.name))
    {
        let quoted = fluxdb_core::MySqlDialect.quote_identifier(&column.name);
        format!("LOWER(SHA2({quoted}, 256)) AS {quoted}")
    } else {
        mysql_select_expr(column)
    }
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

        let columns = mysql_columns_cached(config, &mut connection, database, &path.name).await?;
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

fn mysql_load_cell_text(
    config: &ConnectionConfig,
    path: &ObjectPath,
    identity: &fluxdb_core::RowIdentity,
    column: &str,
) -> fluxdb_core::Result<String> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "仅表和视图支持文本读取",
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
        row.try_get::<Option<String>, _>(0)
            .map_err(mysql_error)?
            .ok_or_else(|| Error::new(ErrorKind::Query, "文本单元格为 NULL"))
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

/// `information_schema.columns` 在远程 MySQL 上单次要 1s 以上（实测 1.4s）。数据同步按块
/// 反复读取同一张表，元数据会盖过真正的取数耗时，因此按「连接 + 库 + 表」缓存一小段时间。
///
/// DDL 之后最多陈旧 `MYSQL_COLUMNS_CACHE_TTL`：期间拼出的 SELECT 列表可能引用已删列而报错，
/// 用户重试即可；这段窗口换来的是大表比对每块少两次元数据往返。
/// key = 连接 ID + 库名 + 表名，value = 写入时刻 + 列定义。
type MysqlColumnsCache = std::sync::Mutex<
    std::collections::HashMap<(u64, String, String), (std::time::Instant, Vec<Column>)>,
>;

static MYSQL_COLUMNS_CACHE: std::sync::LazyLock<MysqlColumnsCache> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

const MYSQL_COLUMNS_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

async fn mysql_columns_cached(
    config: &ConnectionConfig,
    connection: &mut sqlx::MySqlConnection,
    database: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<Column>> {
    let key = (config.id.0, database.to_owned(), table.to_owned());
    let cached = MYSQL_COLUMNS_CACHE
        .lock()
        .ok()
        .and_then(|cache| {
            cache
                .get(&key)
                .filter(|(at, _)| at.elapsed() < MYSQL_COLUMNS_CACHE_TTL)
                .map(|(_, columns)| columns.clone())
        });
    if let Some(columns) = cached {
        return Ok(columns);
    }

    let columns = mysql_columns(connection, database, table).await?;
    // 缓存写失败（锁中毒）只影响性能，不影响本次读取。
    if let Ok(mut cache) = MYSQL_COLUMNS_CACHE.lock() {
        cache.insert(key, (std::time::Instant::now(), columns.clone()));
        if cache.len() > 256 {
            cache.retain(|_, (at, _)| at.elapsed() < MYSQL_COLUMNS_CACHE_TTL);
        }
    }
    Ok(columns)
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

#[cfg(test)]
mod mysql_digest_projection_tests {
    use super::*;

    fn column(name: &str, type_name: &str) -> Column {
        Column {
            name: name.into(),
            type_name: Some(type_name.into()),
            nullable: true,
            primary_key: false,
            comment: None,
        }
    }

    #[test]
    fn digest_column_uses_sha256_and_keeps_alias() {
        let column = column("payload", "longblob");
        let sql = mysql_select_expr_digest(&column, &["PAYLOAD".into()]);
        assert_eq!(sql, "LOWER(SHA2(`payload`, 256)) AS `payload`");
    }

    #[test]
    fn non_digest_column_keeps_page_projection() {
        let column = column("payload", "longblob");
        let sql = mysql_select_expr_digest(&column, &[]);
        assert!(sql.contains("SUBSTRING"), "{sql}");
        assert!(!sql.contains("SHA2"), "{sql}");
    }
}

/// 备份用 MySQL 一致快照：单连接只读事务，DDL 与数据读到同一视图。
///
/// `START TRANSACTION WITH CONSISTENT SNAPSHOT`（REPEATABLE READ）只在 InnoDB
/// 上保证事务快照；MyISAM 等非事务表仍可能读到并发改动，因此 manifest 的
/// `consistent=true` 表示「已建立快照事务」，不宣称所有表都受事务保护。
struct MysqlBackupSnapshot {
    runtime: tokio::runtime::Runtime,
    connection: sqlx::MySqlConnection,
    config: ConnectionConfig,
    finished: bool,
}

impl MysqlBackupSnapshot {
    fn begin(config: &ConnectionConfig) -> fluxdb_core::Result<Self> {
        let plan = mysql_connection_plan(config)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;
        let connection = runtime.block_on(async {
            let mut connection = plan.connect().await?;
            sqlx::raw_sql("SET SESSION TRANSACTION ISOLATION LEVEL REPEATABLE READ")
                .execute(&mut connection)
                .await
                .map_err(mysql_error)?;
            sqlx::raw_sql("START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY")
                .execute(&mut connection)
                .await
                .map_err(mysql_error)?;
            Ok::<_, fluxdb_core::Error>(connection)
        })?;
        Ok(Self {
            runtime,
            connection,
            config: config.clone(),
            finished: false,
        })
    }
}

impl fluxdb_core::BackupSnapshot for MysqlBackupSnapshot {
    fn table_ddl(&mut self, path: &ObjectPath) -> fluxdb_core::Result<String> {
        let database = mysql_database_for_path(&self.config, path)?;
        let sql = format!(
            "SHOW CREATE TABLE {}.{}",
            fluxdb_core::MySqlDialect.quote_identifier(&database),
            fluxdb_core::MySqlDialect.quote_identifier(&path.name)
        );
        let Self {
            runtime,
            connection,
            ..
        } = self;
        runtime.block_on(async {
            let row = sqlx::query(&sql)
                .fetch_one(&mut *connection)
                .await
                .map_err(mysql_error)?;
            row.try_get::<String, _>(1).map_err(mysql_error)
        })
    }

    fn load_data(
        &mut self,
        path: &ObjectPath,
        offset: u64,
        limit: u64,
    ) -> fluxdb_core::Result<DataPage> {
        if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
            return Err(Error::new(ErrorKind::Unsupported, "仅表和视图支持数据读取"));
        }
        let database = path
            .database
            .as_deref()
            .ok_or_else(|| Error::new(ErrorKind::Query, "缺少数据库名称"))?;
        let Self {
            runtime,
            connection,
            config,
            ..
        } = self;
        runtime.block_on(mysql_fetch_page(
            connection,
            config,
            database,
            path,
            offset,
            limit,
            &[],
            &[],
            None,
            &[],
        ))
    }

    fn load_data_keyset(
        &mut self,
        path: &ObjectPath,
        after: &[CellValue],
        limit: u64,
    ) -> fluxdb_core::Result<DataPage> {
        if !matches!(path.kind, ObjectKind::Table) {
            return Err(Error::new(ErrorKind::Unsupported, "仅表支持键集分页"));
        }
        let database = path
            .database
            .as_deref()
            .ok_or_else(|| Error::new(ErrorKind::Query, "缺少数据库名称"))?;
        let Self {
            runtime,
            connection,
            config,
            ..
        } = self;
        runtime.block_on(mysql_fetch_page_keyset(
            connection,
            config,
            database,
            path,
            after,
            limit,
        ))
    }

    fn snapshot_id(&self) -> Option<String> {
        None
    }

    fn finish(&mut self) -> fluxdb_core::Result<()> {
        if self.finished {
            return Ok(());
        }
        let Self {
            runtime,
            connection,
            finished,
            ..
        } = self;
        // 只回滚读事务；连接由结构体 drop 时关闭，避免在 &mut self 上 move 连接。
        runtime.block_on(async {
            sqlx::raw_sql("ROLLBACK")
                .execute(&mut *connection)
                .await
                .map_err(mysql_error)?;
            Ok::<_, fluxdb_core::Error>(())
        })?;
        *finished = true;
        Ok(())
    }
}
