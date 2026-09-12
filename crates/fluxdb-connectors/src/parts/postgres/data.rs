// PostgreSQL 数据读取 / 二进制预览 / 单元格全量读取（T09）。
//
// 与 mysql/sqlite 对齐：`pg_load_data` 走「真实列元数据 + 二进制投影」。
// - 二进制列（bytea 等）在 SELECT 里投影出 3 个派生列（is_null/byte_length/preview_hex），
//   与 mysql 的 `__is_null`/`__byte_length`/`__preview_hex` 语义一致，避免整块取值刷屏；
//   取值时按投影偏移回填 `BinaryCellSummary`。
// - 非二进制列按原生类型返回，交由 `pg_projected_cell_value` 按类型矩阵解码；
//   这样表浏览能看到 I64/Bool/F64/Text/Json 类型化 CellValue，而非一律字符串。
// - 过滤器参数化用 `$n` 占位 + `Vec<&dyn ToSql>` 绑定，杜绝拼接注入。
//
// PG 连接不经过 sqlx QueryBuilder；本文件自行拼 SQL，标识符一律 `pg_quote_identifier`。

use tokio_postgres::types::ToSql;

fn pg_load_data(
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

    pg_runtime().block_on(async {
        let session = pg_connect(config, database).await?;

        let columns = pg_columns(&session.client, database, path.schema.as_deref(), &path.name).await?;
        let (select_list, positions) = pg_select_list_and_positions(&columns);
        let table = pg_qualified_table(database, path.schema.as_deref(), &path.name);

        let mut sql = format!("SELECT {select_list}\nFROM {table}");
        let (where_sql, params) = pg_where_params(filters, &columns)?;
        sql.push_str(&where_sql);
        sql.push_str(&pg_order_by_clause(sort, &columns));
        // limit+1 探测是否还有下一页。
        sql.push_str(&format!(" LIMIT {}", pagination.limit + 1));
        if pagination.offset > 0 {
            sql.push_str(&format!(" OFFSET {}", pagination.offset));
        }

        let rows = tokio::time::timeout(
            Duration::from_secs(30),
            session.client.query(&sql, &pg_params_refs(&params)),
        )
        .await
        .map_err(|_| Error::new(ErrorKind::Query, "查询超时"))?
        .map_err(pg_error)?;

        Ok(pg_data_rows_to_page(columns, rows, positions, pagination))
    })
}

/// 把 `Vec<Box<dyn ToSql>>` 转为 tokio-postgres 需要的 `&[&(dyn ToSql + Sync)]`。
fn pg_params_refs(params: &[Box<dyn ToSql + Sync>]) -> Vec<&(dyn ToSql + Sync)> {
    params.iter().map(|param| param.as_ref()).collect()
}

/// 生成 SELECT 列表并同步返回各列的首投影位置表。
fn pg_select_list_and_positions(columns: &[Column]) -> (String, Vec<(Vec<usize>, usize)>) {
    let mut parts = Vec::with_capacity(columns.len());
    let mut position = 0usize;
    let mut positions = Vec::with_capacity(columns.len());
    for (col_idx, column) in columns.iter().enumerate() {
        let expr = if is_binary_column(column) {
            let name = pg_quote_identifier(&column.name);
            let is_null = pg_quote_identifier(&binary_alias(&column.name, "is_null"));
            let byte_length = pg_quote_identifier(&binary_alias(&column.name, "byte_length"));
            let preview_hex = pg_quote_identifier(&binary_alias(&column.name, "preview_hex"));
            let expr = format!(
                "({name} IS NULL) AS {is_null}, octet_length({name}) AS {byte_length}, \
                 encode(substr({name}, 1, 64), 'hex') AS {preview_hex}"
            );
            positions.push((vec![position, position + 1, position + 2], col_idx));
            position += 3;
            expr
        } else if pg_server_text_type(&column.type_name) {
            // 无法原生解码的类型（money/interval/xml 等）：服务端 `::text` 投影，保真文本。
            let expr = format!(
                "{}::text AS {}",
                pg_quote_identifier(&column.name),
                pg_quote_identifier(&column.name)
            );
            positions.push((vec![position], col_idx));
            position += 1;
            expr
        } else {
            // 保持原生类型，交由类型矩阵解码。
            let expr = format!("{} AS {}", pg_quote_identifier(&column.name), pg_quote_identifier(&column.name));
            positions.push((vec![position], col_idx));
            position += 1;
            expr
        };
        parts.push(expr);
    }
    (parts.join(", "), positions)
}

/// 该列类型无法被驱动原生解码为 String/数值，需在 SELECT 里以 `::text` 服务端投影。
/// numeric/decimal/money 走文本保精度（tokio-postgres 无本地 decimal 解码），interval/xml 及
/// 数组/未知类型同样以服务端文本为唯一保真路径（数组须在剥 `[]` 前判定）。
fn pg_server_text_type(type_name: &Option<String>) -> bool {
    type_name
        .as_deref()
        .map(|t| {
            t.contains('[')
                || t.starts_with('_')
                || matches!(
                    pg_type_base(t),
                    "numeric" | "decimal" | "money" | "interval" | "xml" | "json" | "jsonb"
                )
        })
        .unwrap_or(false)
}

/// 把查询行按投影位置回填为 DataPage；`positions` 描述了每个真实列（下标 → SELECT 投影下标）。
fn pg_data_rows_to_page(
    columns: Vec<Column>,
    rows: Vec<tokio_postgres::Row>,
    positions: Vec<(Vec<usize>, usize)>,
    pagination: Pagination,
) -> DataPage {
    let has_more = rows.len() > pagination.limit as usize;
    let page_rows = rows
        .into_iter()
        .take(pagination.limit as usize)
        .map(|row| Row {
            values: positions
                .iter()
                .map(|(idxs, col_idx)| {
                    pg_projected_position_value(&row, idxs, &columns[*col_idx])
                })
                .collect(),
        })
        .collect();

    DataPage {
        columns,
        rows: page_rows,
        offset: pagination.offset,
        limit: pagination.limit,
        has_more,
    }
}

/// 单列回填：二进制列读 3 个投影列 → BinarySummary；否则用类型矩阵原生解码。
fn pg_projected_position_value(
    row: &tokio_postgres::Row,
    idxs: &[usize],
    column: &Column,
) -> CellValue {
    if is_binary_column(column) {
        let [is_null_idx, byte_length_idx, preview_idx] = idxs else {
            return CellValue::Null;
        };
        let is_null = row
            .try_get::<_, Option<bool>>(*is_null_idx)
            .ok()
            .flatten()
            .unwrap_or(false);
        // octet_length 返回 integer(int4)；按 i64 先试、i32 兜底（驱动类型差异容错）。
        let byte_length = row
            .try_get::<_, Option<i64>>(*byte_length_idx)
            .or_else(|_| {
                row.try_get::<_, Option<i32>>(*byte_length_idx)
                    .map(|v| v.map(|x| x as i64))
            })
            .ok()
            .flatten()
            .unwrap_or(0)
            .max(0) as u64;
        let preview_hex = row
            .try_get::<_, Option<String>>(*preview_idx)
            .ok()
            .flatten()
            .filter(|value| !value.is_empty());
        CellValue::BinarySummary(binary_summary(column, is_null, byte_length, preview_hex))
    } else {
        pg_projected_cell_value(row, idxs[0], column)
    }
}

/// 预览导出：返回行数统计 + 可直接执行的导出 SQL（不落库）。
fn pg_preview_data_export(
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

    let table = pg_qualified_table(database, path.schema.as_deref(), &path.name);

    pg_runtime().block_on(async {
        let session = pg_connect(config, database).await?;
        let columns = pg_columns(&session.client, database, path.schema.as_deref(), &path.name).await?;

        let (where_sql, params) = pg_where_params(filters, &columns)?;
        let count_sql = format!("SELECT count(*) AS c FROM {table}{where_sql}");
        let row = session
            .client
            .query_one(&count_sql, &pg_params_refs(&params))
            .await
            .map_err(pg_error)?;
        let count: i64 = row.get(0);

        Ok(DataExportPreview {
            sql: data_export_preview_sql(&table, fields, sort, filters, &columns, pg_quote_identifier),
            row_count: count.max(0) as u64,
        })
    })
}

/// 全量读取单个二进制单元格（十六进制编辑/文件保存用）。
fn pg_load_cell_binary(
    config: &ConnectionConfig,
    path: &ObjectPath,
    identity: &fluxdb_core::RowIdentity,
    column: &str,
) -> fluxdb_core::Result<Vec<u8>> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(ErrorKind::Unsupported, "仅表和视图支持二进制读取"));
    }
    let database = path
        .database
        .as_deref()
        .ok_or_else(|| Error::new(ErrorKind::Query, "缺少数据库名称"))?;
    let table = pg_qualified_table(database, path.schema.as_deref(), &path.name);

    pg_runtime().block_on(async {
        let session = pg_connect(config, database).await?;
        let columns = pg_columns(&session.client, database, path.schema.as_deref(), &path.name).await?;
        ensure_column_exists(column, &columns)?;

        let mut sql = format!("SELECT {} FROM {table}", pg_quote_identifier(column));
        let (where_sql, params) = pg_identity_params(identity, &columns)?;
        sql.push_str(&where_sql);
        sql.push_str(" LIMIT 1");

        let row = session
            .client
            .query_opt(&sql, &pg_params_refs(&params))
            .await
            .map_err(pg_error)?
            .ok_or_else(|| Error::new(ErrorKind::Query, "未找到目标行"))?;

        row.try_get::<_, Option<Vec<u8>>>(0)
            .map_err(|error| Error::new(ErrorKind::Query, error.to_string()))?
            .ok_or_else(|| Error::new(ErrorKind::Query, "二进制单元格为 NULL"))
    })
}

/// 查询真实列元数据：名字/类型/nullable/主键/注释（T08 已建 pg_columns，复用表结构路径）。
async fn pg_columns(
    client: &tokio_postgres::Client,
    database: &str,
    schema: Option<&str>,
    table: &str,
) -> fluxdb_core::Result<Vec<Column>> {
    let _ = database; // PG 以 schema 定位对象，不依赖目标库名（连接已指定库）。
    let schema_name = schema.unwrap_or("public");
    let rows = client
        .query(
            r#"
            SELECT a.attname AS name,
                   format_type(a.atttypid, a.atttypmod) AS type_name,
                   NOT a.attnotnull AS nullable,
                   COALESCE(a.attnum = ANY(pk.conkey), false) AS primary_key,
                   COALESCE(col_description(a.attrelid, a.attnum), '') AS comment
            FROM pg_attribute a
            JOIN pg_class c ON c.oid = a.attrelid
            JOIN pg_namespace n ON n.oid = c.relnamespace
            LEFT JOIN (
                SELECT conrelid, conkey FROM pg_constraint
                WHERE contype = 'p'
            ) pk ON pk.conrelid = c.oid
            WHERE c.relname = $1 AND n.nspname = $2 AND a.attnum > 0 AND NOT a.attisdropped
            ORDER BY a.attnum
            "#,
            &[&table, &schema_name],
        )
        .await
        .map_err(pg_error)?;

    rows.into_iter()
        .map(|row| {
            Ok(Column {
                name: row.get(0),
                type_name: Some(row.get(1)),
                nullable: row.get(2),
                primary_key: row.get(3),
                comment: Some(row.get::<_, String>(4)).filter(|value| !value.is_empty()),
            })
        })
        .collect::<fluxdb_core::Result<Vec<_>>>()
}

/// 拼限定表名：`"schema"."table"`（schema 默认 public）。
fn pg_qualified_table(database: &str, schema: Option<&str>, table: &str) -> String {
    let _ = database; // 目标库由连接指定；三级限定对 PG 无意义，仅用于一致。
    let schema = schema.unwrap_or("public");
    format!(
        "{}.{}",
        pg_quote_identifier(schema),
        pg_quote_identifier(table)
    )
}

/// 用户排序后面追加主键作为稳定 tie breaker（设计 7.2），避免同值行分页随并发漂移。
/// 主键列若已在用户排序中出现则跳过；无主键时保持共享排序原样。
fn pg_order_by_clause(sort: &[SortSpec], columns: &[Column]) -> String {
    let user_clause = data_order_by_clause(sort, columns, pg_quote_identifier);
    let sorted: Vec<&str> = sort.iter().map(|spec| spec.field.as_str()).collect();
    let tie: Vec<String> = columns
        .iter()
        .filter(|column| column.primary_key && !sorted.contains(&column.name.as_str()))
        .map(|column| pg_quote_identifier(&column.name))
        .collect();
    if tie.is_empty() {
        return user_clause;
    }
    // 用户排序存在则给同一 ORDER BY 追加 tie 列（逗号分隔）；否则新建一个。
    let tie_list = tie.join(", ");
    if user_clause.is_empty() {
        format!(" ORDER BY {tie_list}")
    } else {
        format!("{}, {tie_list}", user_clause.trim_end())
    }
}

/// 拼参数化 WHERE 与绑定参数（`$1..$n`）。
/// 对齐设计 7.2：无法翻译的过滤（列不存在、缺值操作）显式报错，不静默忽略。
fn pg_where_params(filters: &[FilterSpec], columns: &[Column]) -> fluxdb_core::Result<(String, Vec<Box<dyn ToSql + Sync>>)> {
    let mut params: Vec<Box<dyn ToSql + Sync>> = Vec::new();
    let mut clauses = Vec::new();
    for filter in filters.iter().filter(|filter| filter.enabled) {
        if !data_filter_clause_is_pushable(filter) {
            // 缺值操作（空 IN、缺比较值/BETWEEN 端点）属于非法过滤，报错而非丢弃。
            return Err(filter_clause_error(filter));
        }
        let column = columns
            .iter()
            .find(|column| column.name == filter.field)
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Query,
                    format!("过滤引用了不存在的列: {}", filter.field),
                )
            })?;
        clauses.push(pg_filter_clause(filter, column, &mut params)?);
    }
    if clauses.is_empty() {
        Ok((String::new(), params))
    } else {
        Ok((format!("\nWHERE {}", clauses.join("\n  AND ")), params))
    }
}

/// 缺值过滤的明确错误（对齐「非法操作不静默忽略」）。
fn filter_clause_error(filter: &FilterSpec) -> fluxdb_core::Error {
    let op = format!("{:?}", filter.op).to_uppercase();
    Error::new(
        ErrorKind::Query,
        format!("过滤操作 {op} 缺少必要的比较值"),
    )
}

/// 单条过滤器 → 带 `$n` 的 SQL 子句；并收集绑定参数。
/// 由 `pg_where_params` 保证：值类过滤（Between/InList/比较）已带足够值，此处不再判空。
fn pg_filter_clause(
    filter: &FilterSpec,
    column: &Column,
    params: &mut Vec<Box<dyn ToSql + Sync>>,
) -> fluxdb_core::Result<String> {
    let col = column;
    let column = pg_quote_identifier(&column.name);
    Ok(match filter.op {
        FilterOp::IsNull | FilterOp::NotExists => format!("{column} IS NULL"),
        FilterOp::IsNotNull | FilterOp::Exists => format!("{column} IS NOT NULL"),
        FilterOp::IsEmpty => format!("{column} = {}", pg_next_param(params, &CellValue::Text(String::new()), None)),
        FilterOp::IsNotEmpty => {
            format!("{column} != {}", pg_next_param(params, &CellValue::Text(String::new()), None))
        }
        FilterOp::Between | FilterOp::NotBetween => {
            let (start, end) = (&filter.values[0], &filter.values[1]);
            let neg = if filter.op == FilterOp::NotBetween { " NOT" } else { "" };
            format!(
                "{column}{neg} BETWEEN {} AND {}",
                pg_bind_sql(params, col, start),
                pg_bind_sql(params, col, end)
            )
        }
        FilterOp::InList | FilterOp::NotInList => {
            let neg = if filter.op == FilterOp::NotInList { " NOT" } else { "" };
            let placeholders = filter
                .values
                .iter()
                .map(|value| pg_bind_sql(params, col, value))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{column}{neg} IN ({placeholders})")
        }
        FilterOp::Eq | FilterOp::NotEq => {
            let op = if filter.op == FilterOp::Eq { "=" } else { "!=" };
            format!("{column} {op} {}", pg_bind_sql(params, col, &filter.values[0]))
        }
        FilterOp::Contains
        | FilterOp::NotContains
        | FilterOp::StartsWith
        | FilterOp::NotStartsWith
        | FilterOp::EndsWith
        | FilterOp::NotEndsWith => {
            let neg = matches!(
                filter.op,
                FilterOp::NotContains | FilterOp::NotStartsWith | FilterOp::NotEndsWith
            );
            let pattern = match filter.op {
                FilterOp::Contains | FilterOp::NotContains => format!("%{}%", pg_text(&filter.values[0])),
                FilterOp::StartsWith | FilterOp::NotStartsWith => format!("{}%", pg_text(&filter.values[0])),
                FilterOp::EndsWith | FilterOp::NotEndsWith => format!("%{}", pg_text(&filter.values[0])),
                _ => unreachable!(),
            };
            let param = pg_next_param(params, &CellValue::Text(pattern), None);
            format!("{column}{} LIKE {param}", if neg { " NOT" } else { "" })
        }
        FilterOp::GreaterThan
        | FilterOp::GreaterThanOrEqual
        | FilterOp::LessThan
        | FilterOp::LessThanOrEqual => {
            let op = match filter.op {
                FilterOp::GreaterThan => ">",
                FilterOp::GreaterThanOrEqual => ">=",
                FilterOp::LessThan => "<",
                FilterOp::LessThanOrEqual => "<=",
                _ => unreachable!(),
            };
            format!("{column} {op} {}", pg_bind_sql(params, col, &filter.values[0]))
        }
    })
}

/// 追加绑定参数并返回其 `$n` 占位。NULL 以 `Option::<String>::None` 绑定。
///
/// `int_kind` 按目标列宽度选型：`I64` 对 int4/int2 列须降宽（i64 序列化为 int8，列不匹配即失败）。
fn pg_next_param(
    params: &mut Vec<Box<dyn ToSql + Sync>>,
    value: &CellValue,
    int_kind: Option<PgIntKind>,
) -> String {
    let index = params.len() + 1;
    match value {
        CellValue::Null => params.push(Box::new(Option::<String>::None)),
        CellValue::Bool(value) => params.push(Box::new(*value)),
        CellValue::I64(value) => match int_kind {
            Some(PgIntKind::Small) => params.push(Box::new(*value as i16)),
            Some(PgIntKind::Integer) => params.push(Box::new(*value as i32)),
            Some(PgIntKind::Big) | None => params.push(Box::new(*value)),
        },
        CellValue::F64(value) => params.push(Box::new(*value)),
        CellValue::Text(value) | CellValue::Json(value) => params.push(Box::new(value.clone())),
        // 文件/预览二进制：真实内容走 Bytes；BinarySummary 只读预览不参与过滤筛选内容。
        CellValue::Bytes(value) => params.push(Box::new(value.clone())),
        CellValue::BinarySummary(_) => params.push(Box::new(Option::<String>::None)),
    }
    format!("${index}")
}

/// 文本值是否是字符串列的占位（无需 `CAST`）。
/// 数组（`text[]`/`_text`）不属于字符串列：其占位参数被推断为数组类型，String 无法直绑，需转成
/// `text[]` 再让服务端转换，故此处对数组返回 false。
fn pg_text_ok_column(column_type: &str) -> bool {
    if column_type.contains('[') || column_type.starts_with('_') {
        return false;
    }
    matches!(
        pg_type_base(column_type),
        "text" | "varchar" | "character varying" | "char" | "character" | "bpchar" | "name"
            | "citext"
    )
}

/// 值为 Text/Json 且目标列非字符串列时，须**双重转换**占位：
/// - 内层 `CAST($n AS text)` 让 PG 推断 $n 为 text，从而通过客户端 String ToSql 校验
///   （tokio-postgres 无 numeric/bigdecimal 解码，文本无法直绑 numeric/jsonb/数组列）；
/// - 外层 `AS <type>` 在服务端把 text 转成目标列类型，完成赋值。
fn pg_bind_sql(params: &mut Vec<Box<dyn ToSql + Sync>>, column: &Column, value: &CellValue) -> String {
    let param = pg_next_param(params, value, pg_column_int_kind(column));
    let is_text = matches!(value, CellValue::Text(_) | CellValue::Json(_));
    let needs_cast = column
        .type_name
        .as_deref()
        .is_some_and(|t| !pg_text_ok_column(t));
    if is_text && needs_cast {
        let ty = column.type_name.as_deref().unwrap_or("text");
        format!("CAST(CAST({param} AS text) AS {ty})")
    } else {
        param
    }
}

/// 行身份 → 参数化 WHERE；NULL 身份用 `IS NULL`。
fn pg_identity_params(
    identity: &fluxdb_core::RowIdentity,
    columns: &[Column],
) -> fluxdb_core::Result<(String, Vec<Box<dyn ToSql + Sync>>)> {
    let mut params: Vec<Box<dyn ToSql + Sync>> = Vec::new();
    let mut clauses = Vec::new();
    for (column, value) in &identity.values {
        let col = columns
            .iter()
            .find(|col| &col.name == column)
            .ok_or_else(|| Error::new(ErrorKind::Query, "行身份引用了不存在的列"))?;
        let quoted = pg_quote_identifier(column);
        if matches!(value, CellValue::Null) {
            clauses.push(format!("{quoted} IS NULL"));
        } else {
            clauses.push(format!("{quoted} = {}", pg_bind_sql(&mut params, col, value)));
        }
    }
    if clauses.is_empty() {
        return Err(Error::new(ErrorKind::Query, "缺少行身份条件，已取消提交"));
    }
    Ok((format!(" WHERE {}", clauses.join(" AND ")), params))
}

fn pg_text(value: &CellValue) -> String {
    match value {
        CellValue::Text(value) | CellValue::Json(value) => value.clone(),
        other => other.display_label(),
    }
}

/// PG 一致快照导出（T23）：在**单个** REPEATABLE READ 事务内分页读取整表，回调逐页写出。
///
/// 相比桌面按 `load_data` 逐页各开新会话（并发写会让行在页间漂移），本函数在同一事务快照下
/// 分页（`LIMIT..OFFSET` + 主键 tie-breaker 稳定序），保证全量一致且内存有界（一次只持一页）；
/// 每页后检测 `on_cancel`，取消即终止并释放事务（回滚）。`on_page` 返回 false 表示提前停止。
/// 为纯连接器能力（可对隔离库真库验证），供导出驱动接线。
#[allow(clippy::too_many_arguments)]
pub fn pg_export_pages(
    config: &ConnectionConfig,
    path: &ObjectPath,
    sort: &[SortSpec],
    filters: &[FilterSpec],
    cancel: &dyn Fn() -> bool,
    on_page: &mut dyn FnMut(DataPage) -> bool,
) -> fluxdb_core::Result<()> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(ErrorKind::Unsupported, "仅表和视图支持导出"));
    }
    let database = path
        .database
        .as_deref()
        .ok_or_else(|| Error::new(ErrorKind::Query, "缺少数据库名称"))?;
    pg_runtime().block_on(async {
        let session = pg_connect(config, database).await?;
        let client = session.client.as_ref();
        // 单一 REPEATABLE READ 快照：整个导出看到一致视图；导出结束/取消时连接释放即回滚。
        client
            .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ")
            .await
            .map_err(pg_error)?;
        let result: fluxdb_core::Result<()> = async {
            let columns = pg_columns(client, database, path.schema.as_deref(), &path.name).await?;
            let (select_list, positions) = pg_select_list_and_positions(&columns);
            let table = pg_qualified_table(database, path.schema.as_deref(), &path.name);
            let (where_sql, params) = pg_where_params(filters, &columns)?;
            let order = pg_order_by_clause(sort, &columns);
            // 有排序键时逐页 OFFSET；无用户排序时靠主键 tie-breaker 保持稳定序（见 pg_order_by_clause）。
            let mut offset: u64 = 0;
            let batch: u64 = 4096;
            loop {
                if cancel() {
                    return Ok(());
                }
                // 用 offset 游标；每页取多一行探测 has_more。
                let sql = format!(
                    "SELECT {select_list}\nFROM {table}{where_sql}{order} LIMIT {} OFFSET {}",
                    batch + 1,
                    offset
                );
                let rows = client.query(&sql, &pg_params_refs(&params)).await.map_err(pg_error)?;
                let has_more = rows.len() > batch as usize;
                let count = rows.len().min(batch as usize);
                let truncated = rows.into_iter().take(count).collect::<Vec<_>>();
                let page = pg_data_rows_to_page(
                    columns.clone(),
                    truncated,
                    positions.clone(),
                    Pagination::new(offset, count as u64),
                );
                if !on_page(page) {
                    return Ok(());
                }
                offset += count as u64;
                if !has_more || count == 0 {
                    return Ok(());
                }
            }
        }
        .await;
        let _ = client.batch_execute("ROLLBACK").await;
        result
    })
}
