// PostgreSQL 补全元数据（T14）。
//
// 从 pg_catalog 读取 tables/columns/routines/triggers 供补全索引使用。与列表/元数据查询一致：
// 元数据取自独立会话（`pg_connect` 新拨），不干扰用户事务；值一律 `$n` 参数化，标识符不拼接。
// 列读取按列头语义（complete_rel_oid 用 relkind 过滤表/视图），type/schema/comment 一次取齐。

/// 解析补全所用物理数据库。
fn pg_completion_database(
    config: &ConnectionConfig,
    database: Option<&str>,
) -> fluxdb_core::Result<String> {
    match database.filter(|database| !database.trim().is_empty()) {
        Some(database) => Ok(database.to_string()),
        // 缺省回退到档案维护库（同执行路径），保证无显式库时也能连上。
        None => config
            .postgres_profile
            .as_ref()
            .map(|profile| profile.maintenance_database().to_string())
            .filter(|database| !database.trim().is_empty())
            .ok_or_else(|| Error::new(ErrorKind::Connection, "缺少数据库上下文")),
    }
}

/// 解析有效 schema：显式给定优先；否则用连接档案默认 schema，再否则回退 `public`。
fn pg_completion_schema(config: &ConnectionConfig, schema: Option<&str>) -> String {
    if let Some(schema) = schema.filter(|schema| !schema.trim().is_empty()) {
        return schema.to_string();
    }
    config
        .postgres_profile
        .as_ref()
        .and_then(|profile| {
            let scope = &profile.scope;
            (!scope.default_schema.is_empty()).then(|| scope.default_schema.clone())
        })
        .unwrap_or_else(|| "public".to_string())
}

/// 兼容查询的表/视图（relkind：r 普通表、v/m 视图/物化视图、p 分区表、f 外部表），
/// 以常数 IN 列表内联，不绑定数组避免类型推断问题。
/// 构造 LIKE 过滤的 PG 通配符（ESCAPE 反斜杠写法与 completion_fuzzy_like_filter 一致）。
fn pg_completion_like_pattern(filter: &str) -> String {
    let filter = filter.trim().to_ascii_lowercase();
    if filter.is_empty() {
        return "%".to_string();
    }
    let mut pattern = String::from("%");
    for ch in filter.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(ch);
        pattern.push('%');
    }
    pattern
}

async fn pg_list_completion_tables_async(
    client: &tokio_postgres::Client,
    physical_db: &str,
    schema: &str,
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionTable>> {
    let pattern = pg_completion_like_pattern(filter);
    let sql = format!(
        "SELECT c.relname, c.relkind::text \
         FROM pg_catalog.pg_class c \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND c.relkind IN ('r','v','m','p','f') \
           AND c.relname ILIKE $2 ESCAPE '\\' \
         ORDER BY c.relname \
         LIMIT $3"
    );
    let rows = client
        .query(&sql, &[&schema, &pattern, &(limit as i64)])
        .await
        .map_err(pg_error)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let name: String = row.get(0);
            let relkind: String = row.get(1);
            CompletionTable {
                database: Some(physical_db.to_string()),
                schema: Some(schema.to_string()),
                kind: if matches!(relkind.as_str(), "v" | "m") {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                },
                name,
            }
        })
        .collect())
}

/// 批量列：一次 catalog 查询按真实表范围取列，避免逐表 N+1（§8.4）。
async fn pg_list_completion_columns_async(
    client: &tokio_postgres::Client,
    physical_db: &str,
    schema: &str,
    tables: &[&str],
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    if tables.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT c.relname, a.attname, pg_catalog.format_type(a.atttypid, a.atttypmod), \
                a.attnotnull, COALESCE(pg_catalog.col_description(a.attrelid, a.attnum), ''), \
                EXISTS ( \
                  SELECT 1 FROM pg_catalog.pg_index i \
                  WHERE i.indrelid = c.oid AND i.indisprimary AND a.attnum = ANY(i.indkey) \
                ) \
         FROM pg_catalog.pg_class c \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid \
         WHERE n.nspname = $1 AND c.relname = ANY($2::text[]) \
           AND a.attnum > 0 AND NOT a.attisdropped \
         ORDER BY c.relname, a.attnum"
    );
    let table_slice: Vec<&str> = tables.to_vec();
    let rows = client
        .query(
            &sql,
            &[
                &schema,
                &table_slice,
            ],
        )
        .await
        .map_err(pg_error)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let table: String = row.get(0);
            let name: String = row.get(1);
            // format_type 已含 schema 声明与长度，直接作为 type_name 展示。
            let type_name: String = row.get(2);
            let not_null: bool = row.get(3);
            let comment: String = row.get(4);
            let primary_key: bool = row.get(5);
            CompletionColumn {
                database: Some(physical_db.to_string()),
                schema: Some(schema.to_string()),
                table,
                name,
                type_name: Some(type_name),
                nullable: !not_null,
                primary_key,
                comment: (!comment.trim().is_empty()).then_some(comment),
            }
        })
        .collect())
}

async fn pg_list_completion_routines_async(
    client: &tokio_postgres::Client,
    schema: &str,
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
    let pattern = pg_completion_like_pattern(filter);
    let rows = client
        .query(
            "SELECT p.proname, p.prokind::text \
             FROM pg_catalog.pg_proc p \
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
             WHERE n.nspname = $1 AND p.proname ILIKE $2 ESCAPE '\\' \
             ORDER BY p.proname \
             LIMIT $3",
            &[&schema, &pattern, &(limit as i64)],
        )
        .await
        .map_err(pg_error)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let name: String = row.get(0);
            let prokind: String = row.get(1);
            CompletionRoutine {
                schema: Some(schema.to_string()),
                // prokind：f 函数、p 过程、a 聚合、w 窗口（聚合/窗口按函数处理）。
                kind: if prokind == "p" {
                    CompletionRoutineKind::Procedure
                } else {
                    CompletionRoutineKind::Function
                },
                name,
            }
        })
        .collect())
}

async fn pg_list_completion_triggers_async(
    client: &tokio_postgres::Client,
    schema: &str,
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
    let pattern = pg_completion_like_pattern(filter);
    let rows = client
        .query(
            "SELECT t.tgname, c.relname \
             FROM pg_catalog.pg_trigger t \
             JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND NOT t.tgisinternal \
               AND t.tgname ILIKE $2 ESCAPE '\\' \
             ORDER BY t.tgname \
             LIMIT $3",
            &[&schema, &pattern, &(limit as i64)],
        )
        .await
        .map_err(pg_error)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let name: String = row.get(0);
            let table: String = row.get(1);
            CompletionTrigger {
                schema: Some(schema.to_string()),
                name,
                table: Some(table),
            }
        })
        .collect())
}

fn pg_list_completion_tables(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionTable>> {
    let physical_db = pg_completion_database(config, database)?;
    let schema = pg_completion_schema(config, schema);
    pg_runtime().block_on(async {
        let session = pg_connect(config, &physical_db).await?;
        pg_list_completion_tables_async(
            session.client.as_ref(),
            &physical_db,
            &schema,
            filter,
            limit,
        )
        .await
    })
}

fn pg_list_completion_columns(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    table: &str,
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    let physical_db = pg_completion_database(config, database)?;
    let schema = pg_completion_schema(config, schema);
    pg_runtime().block_on(async {
        let session = pg_connect(config, &physical_db).await?;
        pg_list_completion_columns_async(
            session.client.as_ref(),
            &physical_db,
            &schema,
            &[table],
        )
        .await
    })
}

fn pg_list_completion_columns_for_tables(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    tables: &[String],
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    let physical_db = pg_completion_database(config, database)?;
    let schema = pg_completion_schema(config, schema);
    let table_refs: Vec<&str> = tables.iter().map(String::as_str).collect();
    pg_runtime().block_on(async {
        let session = pg_connect(config, &physical_db).await?;
        pg_list_completion_columns_async(session.client.as_ref(), &physical_db, &schema, &table_refs)
            .await
    })
}

fn pg_list_completion_routines(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
    let physical_db = pg_completion_database(config, database)?;
    let schema = pg_completion_schema(config, schema);
    pg_runtime().block_on(async {
        let session = pg_connect(config, &physical_db).await?;
        pg_list_completion_routines_async(session.client.as_ref(), &schema, filter, limit).await
    })
}

fn pg_list_completion_triggers(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
    let physical_db = pg_completion_database(config, database)?;
    let schema = pg_completion_schema(config, schema);
    pg_runtime().block_on(async {
        let session = pg_connect(config, &physical_db).await?;
        pg_list_completion_triggers_async(session.client.as_ref(), &schema, filter, limit).await
    })
}
