// Oracle 数据页：筛选、排序、分页、导出预览。
//
// 分页一律 `OFFSET … ROWS FETCH NEXT … ROWS ONLY`（12c+，docs/oracle.md §6）；
// 它要求 ORDER BY，因此没有显式排序时按主键补齐，再退化到首列——分页稳定靠排序，
// 不靠 ROWNUM 的偶然顺序。筛选统一走编译后的表达式（`:N` 绑定），不拼接值。

/// 多取一行只用于判断 `has_more`，不展示给调用方。
fn oracle_page_fetch_limit(limit: u64) -> u64 {
    limit.saturating_add(1)
}

/// 按行身份读取单元格完整文本（CLOB 详情/编辑用）。
///
/// 数据页只装预览，这里必须整段读完：拿预览当完整值写回会丢数据。读取上限见
/// [`ORACLE_LOB_TEXT_FULL_MAX_BYTES`]，超限报错而不是截断。
fn oracle_load_cell_text(
    config: &ConnectionConfig,
    path: &ObjectPath,
    identity: &fluxdb_core::RowIdentity,
    column: &str,
) -> fluxdb_core::Result<String> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(ErrorKind::Unsupported, "仅表和视图支持文本读取"));
    }
    let schema = oracle_request_schema(config, path)?;
    let connection = oracle_connect(config)?;
    let columns = oracle_load_columns(&connection, &schema, &path.name)?;
    ensure_column_exists(column, &columns)?;
    let target = columns
        .iter()
        .find(|candidate| candidate.name == column)
        .ok_or_else(|| Error::new(ErrorKind::Query, format!("字段不存在或已变更：{column}")))?;
    let (where_sql, binds) = oracle_identity_where(identity, &columns)?;
    let sql = format!(
        "SELECT {} FROM {}{where_sql} FETCH FIRST 1 ROWS ONLY",
        fluxdb_core::OracleDialect.quote_identifier(column),
        oracle_qualified_table(&schema, &path.name)
    );
    let mut rows = oracle_query(&connection, &sql, &binds)?;
    let row = rows
        .first_mut()
        .ok_or_else(|| Error::new(ErrorKind::Query, "未找到目标行"))?;
    let is_character_lob = row
        .columns()
        .first()
        .is_some_and(|metadata| oracle_db_type_is_character_lob(metadata.db_type()));
    if is_character_lob {
        let Ok(value) = row.take::<Option<OracleLob>>(0) else {
            return Err(Error::new(ErrorKind::Query, "读取 CLOB 失败"));
        };
        let Some(mut lob) = value else {
            return Err(Error::new(ErrorKind::Query, "文本单元格为 NULL"));
        };
        return read_lob_text_unbounded(&mut lob, ORACLE_LOB_TEXT_FULL_MAX_BYTES);
    }
    match oracle_cell_value(row, 0, target) {
        CellValue::Text(text) | CellValue::Json(text) => Ok(text),
        // 非字符 LOB 或二进制列没有「完整文本」语义，不伪造一个。
        _ => Err(Error::new(
            ErrorKind::Unsupported,
            "当前单元格不是文本字段",
        )),
    }
}

/// 二进制完整值读取上限：与文本一致，超限报错而不是截断。
const ORACLE_LOB_BINARY_FULL_MAX_BYTES: usize = 8 * 1024 * 1024;

/// 按行身份读取二进制单元格完整值（同步写回大字段用）。
fn oracle_load_cell_binary(
    config: &ConnectionConfig,
    path: &ObjectPath,
    identity: &fluxdb_core::RowIdentity,
    column: &str,
) -> fluxdb_core::Result<Vec<u8>> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(ErrorKind::Unsupported, "仅表和视图支持二进制读取"));
    }
    let schema = oracle_request_schema(config, path)?;
    let connection = oracle_connect(config)?;
    let columns = oracle_load_columns(&connection, &schema, &path.name)?;
    ensure_column_exists(column, &columns)?;
    let target = columns
        .iter()
        .find(|candidate| candidate.name == column)
        .ok_or_else(|| Error::new(ErrorKind::Query, format!("字段不存在或已变更：{column}")))?;
    let (where_sql, binds) = oracle_identity_where(identity, &columns)?;
    let sql = format!(
        "SELECT {} FROM {}{where_sql} FETCH FIRST 1 ROWS ONLY",
        fluxdb_core::OracleDialect.quote_identifier(column),
        oracle_qualified_table(&schema, &path.name)
    );
    let mut rows = oracle_query(&connection, &sql, &binds)?;
    let row = rows
        .first_mut()
        .ok_or_else(|| Error::new(ErrorKind::Query, "未找到目标行"))?;
    let is_lob = row
        .columns()
        .first()
        .is_some_and(|metadata| oracle_db_type_is_lob(metadata.db_type()));
    if is_lob {
        let Ok(value) = row.take::<Option<OracleLob>>(0) else {
            return Err(Error::new(ErrorKind::Query, "读取 LOB 失败"));
        };
        let Some(mut lob) = value else {
            return Err(Error::new(ErrorKind::Query, "二进制单元格为 NULL"));
        };
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            if bytes.len() >= ORACLE_LOB_BINARY_FULL_MAX_BYTES {
                return Err(Error::new(ErrorKind::Query, "二进制单元格超过 8 MiB，无法完整读入"));
            }
            match std::io::Read::read(&mut lob, &mut chunk) {
                Ok(0) => break,
                Ok(read) => bytes.extend_from_slice(&chunk[..read]),
                Err(error) => {
                    return Err(Error::new(ErrorKind::Query, format!("读取 LOB 失败：{error}")));
                }
            }
        }
        return Ok(bytes);
    }
    match oracle_cell_value(row, 0, target) {
        CellValue::Bytes(bytes) => Ok(bytes),
        CellValue::Text(text) => Ok(text.into_bytes()),
        CellValue::Null => Err(Error::new(ErrorKind::Query, "二进制单元格为 NULL")),
        _ => Err(Error::new(ErrorKind::Query, "二进制单元格类型异常")),
    }
}

fn oracle_load_data(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    filters: &[FilterSpec],
) -> fluxdb_core::Result<DataPage> {
    oracle_load_data_with_expression(config, path, offset, limit, sort, filters, None, &[])
}

fn oracle_load_data_digests(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    filters: &[FilterSpec],
    digest_columns: &[String],
) -> fluxdb_core::Result<DataPage> {
    oracle_load_data_with_expression(
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

fn oracle_load_data_expression(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    expression: &fluxdb_core::FilterExpression,
) -> fluxdb_core::Result<DataPage> {
    // 显式实现真实筛选链路：默认 trait 实现会丢掉 expression，退化成无过滤全表。
    oracle_load_data_with_expression(
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

fn oracle_load_data_with_expression(
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
    let schema = oracle_request_schema(config, path)?;
    let pagination = Pagination::new(offset, limit);
    let connection = oracle_connect(config)?;
    let columns = oracle_load_columns(&connection, &schema, &path.name)?;
    let table = oracle_qualified_table(&schema, &path.name);
    let (where_sql, binds) = match expression {
        Some(expression) => oracle_expression_where(expression, &columns)?,
        None => oracle_filter_where(filters, &columns)?,
    };
    let order = oracle_order_by(sort, &columns);
    let select_list = columns
        .iter()
        .map(|column| fluxdb_core::OracleDialect.quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let fetch = oracle_page_fetch_limit(pagination.limit);
    // OFFSET/FETCH 用字面量而不是绑定：该子句只接受常量，绑定会报 ORA-01036。
    let sql = format!(
        "SELECT {select_list} FROM {table}{where_sql}{order} OFFSET {} ROWS FETCH NEXT {fetch} ROWS ONLY",
        pagination.offset
    );
    let rows = oracle_query(&connection, &sql, &binds)?;
    let mut page_rows = Vec::with_capacity(rows.len());
    for mut row in rows {
        page_rows.push(oracle_row_with_digests(
            &mut row,
            &columns,
            digest_columns,
        ));
    }
    let has_more = page_rows.len() > pagination.limit as usize;
    page_rows.truncate(pagination.limit as usize);
    Ok(DataPage {
        columns,
        rows: page_rows,
        offset: pagination.offset,
        limit: pagination.limit,
        has_more,
        incomplete: None,
    })
}

/// ORDER BY 片段。显式写 `NULLS FIRST/LAST`，让排序结果不依赖会话或版本默认值。
fn oracle_order_by(sort: &[SortSpec], columns: &[Column]) -> String {
    let mut parts = Vec::new();
    for spec in sort {
        if !columns.iter().any(|column| column.name == spec.field) {
            continue;
        }
        let direction = match spec.direction {
            SortDirection::Asc => "ASC NULLS LAST",
            SortDirection::Desc => "DESC NULLS FIRST",
        };
        parts.push(format!(
            "{} {direction}",
            fluxdb_core::OracleDialect.quote_identifier(&spec.field)
        ));
    }
    let sorted = sort.iter().map(|spec| spec.field.as_str()).collect::<Vec<_>>();
    // 主键补齐保证同一排序值内的顺序确定，翻页不会重复/漏行。
    for column in columns
        .iter()
        .filter(|column| column.primary_key && !sorted.contains(&column.name.as_str()))
    {
        parts.push(format!(
            "{} ASC NULLS LAST",
            fluxdb_core::OracleDialect.quote_identifier(&column.name)
        ));
    }
    if parts.is_empty() {
        // 视图或堆表可能没有可用主键：退到首列排序，仍是确定顺序（值相同的行顺序
        // 由存储决定，这一点与其它方言一致，不做伪保证）。
        if let Some(column) = columns.first() {
            parts.push(format!(
                "{} ASC NULLS LAST",
                fluxdb_core::OracleDialect.quote_identifier(&column.name)
            ));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {}", parts.join(", "))
    }
}

fn oracle_expression_where(
    expression: &fluxdb_core::FilterExpression,
    columns: &[Column],
) -> fluxdb_core::Result<(String, Vec<OracleBindValue>)> {
    use fluxdb_core::FilterDialect as _;
    // 走方言编译：占位符按出现顺序生成唯一 `:N`，值只以绑定形式下发。
    let compiled = crate::filter_dialects::oracle::ORACLE_SYNTAX
        .compile_with_columns(expression, columns)
        .map_err(|error| Error::new(ErrorKind::Query, error.message))?;
    if compiled.sql.trim().is_empty() {
        return Ok((String::new(), Vec::new()));
    }
    let binds = compiled
        .parameters
        .iter()
        .map(oracle_bind_filter)
        .collect::<Vec<_>>();
    Ok((format!(" WHERE {}", compiled.sql), binds))
}

fn oracle_filter_where(
    filters: &[FilterSpec],
    columns: &[Column],
) -> fluxdb_core::Result<(String, Vec<OracleBindValue>)> {
    let mut clauses = Vec::new();
    let mut binds = Vec::new();
    for filter in filters.iter().filter(|filter| filter.enabled) {
        if !data_filter_clause_is_pushable(filter) {
            return Err(Error::new(ErrorKind::Query, "Oracle 过滤条件缺少必要的比较值"));
        }
        let Some(column) = columns.iter().find(|column| column.name == filter.field) else {
            return Err(Error::new(
                ErrorKind::Query,
                format!("过滤引用了不存在的列: {}", filter.field),
            ));
        };
        if let Some(clause) = oracle_push_filter(&mut binds, filter, column) {
            clauses.push(clause);
        }
    }
    if clauses.is_empty() {
        Ok((String::new(), binds))
    } else {
        Ok((format!(" WHERE {}", clauses.join(" AND ")), binds))
    }
}

fn oracle_placeholder(binds: &mut Vec<OracleBindValue>, value: OracleBindValue) -> String {
    binds.push(value);
    format!(":{}", binds.len())
}

fn oracle_push_filter(
    binds: &mut Vec<OracleBindValue>,
    filter: &FilterSpec,
    column: &Column,
) -> Option<String> {
    let name = fluxdb_core::OracleDialect.quote_identifier(&column.name);
    Some(match filter.op {
        FilterOp::IsNull | FilterOp::NotExists => format!("{name} IS NULL"),
        FilterOp::IsNotNull | FilterOp::Exists => format!("{name} IS NOT NULL"),
        // Oracle 把空字符串存成 NULL：`= ''` 永远不成立，空值判断必须用 IS NULL。
        FilterOp::IsEmpty => format!("{name} IS NULL"),
        FilterOp::IsNotEmpty => format!("{name} IS NOT NULL"),
        FilterOp::Between | FilterOp::NotBetween => {
            let start = filter.values.first()?;
            let end = filter.values.get(1)?;
            let start = oracle_placeholder(binds, oracle_bind_cell(start));
            let end = oracle_placeholder(binds, oracle_bind_cell(end));
            format!(
                "{name}{} BETWEEN {start} AND {end}",
                if filter.op == FilterOp::NotBetween { " NOT" } else { "" }
            )
        }
        FilterOp::InList | FilterOp::NotInList => {
            if filter.values.is_empty() {
                return None;
            }
            let values = filter
                .values
                .iter()
                .map(|value| oracle_placeholder(binds, oracle_bind_cell(value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{name}{} IN ({values})",
                if filter.op == FilterOp::NotInList { " NOT" } else { "" }
            )
        }
        FilterOp::Eq | FilterOp::NotEq => {
            if filter.values.is_empty() {
                return None;
            }
            // NULL 不能用 `= NULL` 表达，拆成 IS NULL / IS NOT NULL。
            let mut predicates = Vec::new();
            for value in &filter.values {
                let predicate = match (value, filter.op) {
                    (CellValue::Null, FilterOp::Eq) => format!("{name} IS NULL"),
                    (CellValue::Null, _) => format!("{name} IS NOT NULL"),
                    (value, FilterOp::Eq) => format!(
                        "{name} = {}",
                        oracle_placeholder(binds, oracle_bind_cell(value))
                    ),
                    (value, _) => format!(
                        "{name} != {}",
                        oracle_placeholder(binds, oracle_bind_cell(value))
                    ),
                };
                predicates.push(predicate);
            }
            if predicates.len() > 1 {
                format!(
                    "({})",
                    predicates
                        .join(if filter.op == FilterOp::Eq { " OR " } else { " AND " })
                )
            } else {
                predicates.join("")
            }
        }
        FilterOp::Contains
        | FilterOp::NotContains
        | FilterOp::StartsWith
        | FilterOp::NotStartsWith
        | FilterOp::EndsWith
        | FilterOp::NotEndsWith => {
            let value = filter.values.first().map(data_filter_value_text)?;
            let pattern = match filter.op {
                FilterOp::Contains | FilterOp::NotContains => format!("%{value}%"),
                FilterOp::StartsWith | FilterOp::NotStartsWith => format!("{value}%"),
                _ => format!("%{value}"),
            };
            let negative = matches!(
                filter.op,
                FilterOp::NotContains | FilterOp::NotStartsWith | FilterOp::NotEndsWith
            );
            let placeholder = oracle_placeholder(binds, OracleBindValue::Text(pattern));
            format!("{name} {}LIKE {placeholder}", if negative { "NOT " } else { "" })
        }
        FilterOp::GreaterThan
        | FilterOp::GreaterThanOrEqual
        | FilterOp::LessThan
        | FilterOp::LessThanOrEqual => {
            let value = filter.values.first()?;
            let op = match filter.op {
                FilterOp::GreaterThan => ">",
                FilterOp::GreaterThanOrEqual => ">=",
                FilterOp::LessThan => "<",
                _ => "<=",
            };
            let placeholder = oracle_placeholder(binds, oracle_bind_cell(value));
            format!("{name} {op} {placeholder}")
        }
    })
}

/// 导出预览：与真实导出口径一致（显式字段、过滤、排序），只是把绑定渲染成字面量。
///
/// 这是「数据导出 / 逻辑导出」，不是一致性快照：预览 SQL 由客户端拼装，仅供确认范围。
fn oracle_preview_data_export(
    config: &ConnectionConfig,
    path: &ObjectPath,
    fields: &[String],
    sort: &[SortSpec],
    filters: &[FilterSpec],
) -> fluxdb_core::Result<DataExportPreview> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(ErrorKind::Unsupported, "仅表和视图支持导出预览"));
    }
    let schema = oracle_request_schema(config, path)?;
    let connection = oracle_connect(config)?;
    let columns = oracle_load_columns(&connection, &schema, &path.name)?;
    let table = oracle_qualified_table(&schema, &path.name);
    let selected = fields
        .iter()
        .filter_map(|field| columns.iter().find(|column| column.name == *field))
        .map(|column| fluxdb_core::OracleDialect.quote_identifier(&column.name))
        .collect::<Vec<_>>();
    let select_list = if selected.is_empty() {
        "*".to_string()
    } else {
        selected.join(", ")
    };
    let where_preview = data_where_clause_preview(filters, &columns, |value| {
        fluxdb_core::OracleDialect.quote_identifier(value)
    });
    let sql = format!(
        "SELECT {select_list}\nFROM {table}{where_preview}{}",
        oracle_order_by(sort, &columns)
    );
    // 行数用计数查询得到，而不是把数据取回来再数。计数仍走真绑定，
    // 预览里的字面量只用于展示，不拿去执行。
    let (count_where, count_binds) = oracle_filter_where(filters, &columns)?;
    let count_sql = format!("SELECT COUNT(*) FROM {table}{count_where}");
    let rows = oracle_query(&connection, &count_sql, &count_binds)?;
    let row_count = rows
        .first()
        .and_then(|row| oracle_meta_u64(row, 0))
        .unwrap_or(0);
    Ok(DataExportPreview { sql, row_count })
}
