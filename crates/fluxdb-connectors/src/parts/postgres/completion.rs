// PostgreSQL 补全元数据（T14）。
//
// 从 pg_catalog 读取 tables/columns/routines/triggers 供补全索引使用。与列表/元数据查询一致：
// 元数据取自独立会话（`pg_connect` 新拨），不干扰用户事务；值一律 `$n` 参数化，标识符不拼接。
//
// schema 范围（§8.4）：显式 schema 优先（支持显式跨 schema）；未显式给定则读取服务器
// `current_schemas(false)` 的有效 search_path 顺序，补全跟随 PG 的未限定名解析顺序，
// 不硬编码 public。同名对象分属不同 schema 时分别返回，由上层按 schema 消歧。

/// 补全默认回退 schema：无 search_path、无档案默认时的最后兜底。
const PG_COMPLETION_FALLBACK_SCHEMA: &str = "public";

/// 兼容查询的表/视图（relkind：r 普通表、v/m 视图/物化视图、p 分区表、f 外部表），
/// 以常数 IN 列表内联，不绑定数组避免类型推断问题。
const PG_COMPLETION_RELKINDS: &str = "'r','v','m','p','f'";

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

/// 档案默认 schema：连接档案 scope 显式配置的默认 schema（未配置为空串）。
fn pg_profile_default_schema(config: &ConnectionConfig) -> Option<String> {
    config
        .postgres_profile
        .as_ref()
        .and_then(|profile| {
            let scope = &profile.scope;
            (!scope.default_schema.is_empty()).then(|| scope.default_schema.clone())
        })
}

/// 读取服务器有效 search_path（按生效顺序，排除隐式 pg_catalog）。
///
/// PG 未限定标识符按 search_path 顺序解析，补全范围必须跟随该顺序；
/// 读取失败不阻塞补全，返回空表由调用方回退档案默认/public。
async fn pg_effective_search_path_async(client: &tokio_postgres::Client) -> Vec<String> {
    match client
        .query_one("SELECT pg_catalog.current_schemas(false)", &[])
        .await
    {
        Ok(row) => {
            let schemas: Vec<String> = row.get(0);
            schemas
                .into_iter()
                .filter(|schema| !schema.trim().is_empty())
                .collect()
        }
        Err(error) => {
            tracing::warn!(
                target: "fluxdb_connectors",
                error = %error,
                "读取 search_path 失败，回退档案默认 schema"
            );
            Vec::new()
        }
    }
}

/// 解析本次补全的 schema 范围。
///
/// 显式 schema → 仅该 schema（显式跨 schema）；否则服务器 search_path 顺序；
/// 再否则档案默认 schema；最后回退 `public`。
async fn pg_completion_schemas_async(
    client: &tokio_postgres::Client,
    config: &ConnectionConfig,
    schema: Option<&str>,
) -> Vec<String> {
    if let Some(schema) = schema.filter(|schema| !schema.trim().is_empty()) {
        return vec![schema.to_string()];
    }
    let effective = pg_effective_search_path_async(client).await;
    if !effective.is_empty() {
        return effective;
    }
    vec![pg_profile_default_schema(config)
        .unwrap_or_else(|| PG_COMPLETION_FALLBACK_SCHEMA.to_string())]
}

/// 构造 LIKE 过滤的 PG 通配符（ESCAPE 反斜杠写法与 completion_fuzzy_like_filter 一致）。
///
/// 未加引号的 SQL 标识符按 PG 语义折叠为小写，模糊匹配统一折小写；
/// catalog 返回名称仍按原样持有，不做大小写合并。
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

/// 按 schema 优先级保留每个表名首个（search_path 最靠前）schema 的行。
///
/// PG 未限定 `t.col` 按 search_path 顺序解析，故同表名跨 schema 时只有首个可见；
/// 显式指定 schema 时（范围长度 1）为无操作。
fn pg_keep_first_visible_schema(rows: &mut Vec<(String, String)>, schemas: &[String]) {
    if schemas.len() <= 1 {
        return;
    }
    let mut chosen: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (table, schema) in rows.iter() {
        let rank = schemas
            .iter()
            .position(|candidate| candidate == schema)
            .unwrap_or(usize::MAX);
        chosen
            .entry(table.clone())
            .and_modify(|current| *current = (*current).min(rank))
            .or_insert(rank);
    }
    rows.retain(|(table, schema)| {
        let rank = schemas
            .iter()
            .position(|candidate| candidate == schema)
            .unwrap_or(usize::MAX);
        chosen.get(table) == Some(&rank)
    });
}

async fn pg_list_completion_tables_async(
    client: &tokio_postgres::Client,
    physical_db: &str,
    schemas: &[String],
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionTable>> {
    let pattern = pg_completion_like_pattern(filter);
    // ORDER BY array_position：search_path 靠前的 schema 先出，符合 PG 可见性优先级。
    let sql = format!(
        "SELECT c.relname, c.relkind::text, n.nspname, \
                COALESCE(pg_catalog.obj_description(c.oid, 'pg_class'), '') \
         FROM pg_catalog.pg_class c \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = ANY($1::text[]) AND c.relkind IN ({PG_COMPLETION_RELKINDS}) \
           AND c.relname ILIKE $2 ESCAPE '\\' \
         ORDER BY array_position($1::text[], n.nspname), c.relname \
         LIMIT $3"
    );
    let schema_slice: Vec<&str> = schemas.iter().map(String::as_str).collect();
    let rows = client
        .query(&sql, &[&schema_slice, &pattern, &(limit as i64)])
        .await
        .map_err(pg_error)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let name: String = row.get(0);
            let relkind: String = row.get(1);
            let schema: String = row.get(2);
            let comment: String = row.get(3);
            CompletionTable {
                database: Some(physical_db.to_string()),
                schema: Some(schema),
                kind: if matches!(relkind.as_str(), "v" | "m") {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                },
                name,
                comment: (!comment.trim().is_empty()).then_some(comment),
            }
        })
        .collect())
}

/// 批量列：一次 catalog 查询按真实表范围取列，避免逐表 N+1（§8.4）。
///
/// 同表名跨 schema 时按 search_path 顺序只保留首个可见 schema 的列（见
/// `pg_keep_first_visible_schema`），与未限定名的解析结果一致，避免跨 schema 串列。
async fn pg_list_completion_columns_async(
    client: &tokio_postgres::Client,
    physical_db: &str,
    schemas: &[String],
    tables: &[&str],
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    if tables.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT c.relname, n.nspname, a.attname, \
                pg_catalog.format_type(a.atttypid, a.atttypmod), \
                a.attnotnull, COALESCE(pg_catalog.col_description(a.attrelid, a.attnum), ''), \
                EXISTS ( \
                  SELECT 1 FROM pg_catalog.pg_index i \
                  WHERE i.indrelid = c.oid AND i.indisprimary AND a.attnum = ANY(i.indkey) \
                ), a.attnum::int4 \
         FROM pg_catalog.pg_class c \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid \
         WHERE n.nspname = ANY($1::text[]) AND c.relname = ANY($2::text[]) \
           AND a.attnum > 0 AND NOT a.attisdropped \
         ORDER BY c.relname, array_position($1::text[], n.nspname), a.attnum"
    );
    let schema_slice: Vec<&str> = schemas.iter().map(String::as_str).collect();
    let table_slice: Vec<&str> = tables.to_vec();
    let rows = client
        .query(&sql, &[&schema_slice, &table_slice])
        .await
        .map_err(pg_error)?;

    // 先做可见性裁剪，再转领域结构：裁剪只需要 (table, schema) 对。
    let mut pairs: Vec<(String, String)> = rows
        .iter()
        .map(|row| (row.get::<_, String>(0), row.get::<_, String>(1)))
        .collect();
    pg_keep_first_visible_schema(&mut pairs, schemas);
    let kept: std::collections::BTreeSet<(String, String)> = pairs.into_iter().collect();

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let table: String = row.get(0);
            let schema: String = row.get(1);
            if !kept.contains(&(table.clone(), schema.clone())) {
                return None;
            }
            let name: String = row.get(2);
            // format_type 已含 schema 声明与长度，直接作为 type_name 展示。
            let type_name: String = row.get(3);
            let not_null: bool = row.get(4);
            let comment: String = row.get(5);
            let primary_key: bool = row.get(6);
            let attnum: i32 = row.get(7);
            Some(CompletionColumn {
                database: Some(physical_db.to_string()),
                schema: Some(schema),
                table,
                name,
                type_name: Some(type_name),
                nullable: !not_null,
                primary_key,
                comment: (!comment.trim().is_empty()).then_some(comment),
                stable: u64::try_from(attnum).ok(),
            })
        })
        .collect())
}

async fn pg_list_completion_routines_async(
    client: &tokio_postgres::Client,
    schemas: &[String],
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
    let pattern = pg_completion_like_pattern(filter);
    // 签名（identity arguments）区分同 schema 同名重载，补全/索引按签名分条（§8.4）。
    let sql = format!(
        "SELECT p.proname, p.prokind::text, n.nspname, \
                pg_catalog.pg_get_function_identity_arguments(p.oid) \
         FROM pg_catalog.pg_proc p \
         JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
         WHERE n.nspname = ANY($1::text[]) AND p.proname ILIKE $2 ESCAPE '\\' \
         ORDER BY array_position($1::text[], n.nspname), p.proname, \
                  pg_catalog.pg_get_function_identity_arguments(p.oid) \
         LIMIT $3"
    );
    let schema_slice: Vec<&str> = schemas.iter().map(String::as_str).collect();
    let rows = client
        .query(&sql, &[&schema_slice, &pattern, &(limit as i64)])
        .await
        .map_err(pg_error)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let name: String = row.get(0);
            let prokind: String = row.get(1);
            let schema: String = row.get(2);
            let signature: String = row.get(3);
            CompletionRoutine {
                schema: Some(schema),
                // prokind：f 函数、p 过程、a 聚合、w 窗口（聚合/窗口按函数处理）。
                kind: if prokind == "p" {
                    CompletionRoutineKind::Procedure
                } else {
                    CompletionRoutineKind::Function
                },
                name,
                signature: Some(signature),
            }
        })
        .collect())
}

async fn pg_list_completion_triggers_async(
    client: &tokio_postgres::Client,
    schemas: &[String],
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
    let pattern = pg_completion_like_pattern(filter);
    let sql = format!(
        "SELECT t.tgname, c.relname, n.nspname \
         FROM pg_catalog.pg_trigger t \
         JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = ANY($1::text[]) AND NOT t.tgisinternal \
           AND t.tgname ILIKE $2 ESCAPE '\\' \
         ORDER BY array_position($1::text[], n.nspname), t.tgname \
         LIMIT $3"
    );
    let schema_slice: Vec<&str> = schemas.iter().map(String::as_str).collect();
    let rows = client
        .query(&sql, &[&schema_slice, &pattern, &(limit as i64)])
        .await
        .map_err(pg_error)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let name: String = row.get(0);
            let table: String = row.get(1);
            let schema: String = row.get(2);
            CompletionTrigger {
                schema: Some(schema),
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
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionTable>> {
    let physical_db = pg_completion_database(config, database)?;
    pg_runtime().block_on(async {
        let session = pg_connect(config, &physical_db).await?;
        let schemas = pg_completion_schemas_async(session.client.as_ref(), config, schema).await;
        // 取消检查：建连/search_path 取完之后、主 catalog 查询之前（§8.4 列表可取消）。
        if should_cancel() {
            return Ok(Vec::new());
        }
        pg_list_completion_tables_async(
            session.client.as_ref(),
            &physical_db,
            &schemas,
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
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    let physical_db = pg_completion_database(config, database)?;
    pg_runtime().block_on(async {
        let session = pg_connect(config, &physical_db).await?;
        let schemas = pg_completion_schemas_async(session.client.as_ref(), config, schema).await;
        if should_cancel() {
            return Ok(Vec::new());
        }
        pg_list_completion_columns_async(
            session.client.as_ref(),
            &physical_db,
            &schemas,
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
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    let physical_db = pg_completion_database(config, database)?;
    let table_refs: Vec<&str> = tables.iter().map(String::as_str).collect();
    pg_runtime().block_on(async {
        let session = pg_connect(config, &physical_db).await?;
        let schemas = pg_completion_schemas_async(session.client.as_ref(), config, schema).await;
        if should_cancel() {
            return Ok(Vec::new());
        }
        pg_list_completion_columns_async(session.client.as_ref(), &physical_db, &schemas, &table_refs)
            .await
    })
}

fn pg_list_completion_routines(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    filter: &str,
    limit: u64,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
    let physical_db = pg_completion_database(config, database)?;
    pg_runtime().block_on(async {
        let session = pg_connect(config, &physical_db).await?;
        let schemas = pg_completion_schemas_async(session.client.as_ref(), config, schema).await;
        if should_cancel() {
            return Ok(Vec::new());
        }
        pg_list_completion_routines_async(session.client.as_ref(), &schemas, filter, limit).await
    })
}

fn pg_list_completion_triggers(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    filter: &str,
    limit: u64,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
    let physical_db = pg_completion_database(config, database)?;
    pg_runtime().block_on(async {
        let session = pg_connect(config, &physical_db).await?;
        let schemas = pg_completion_schemas_async(session.client.as_ref(), config, schema).await;
        if should_cancel() {
            return Ok(Vec::new());
        }
        pg_list_completion_triggers_async(session.client.as_ref(), &schemas, filter, limit).await
    })
}
