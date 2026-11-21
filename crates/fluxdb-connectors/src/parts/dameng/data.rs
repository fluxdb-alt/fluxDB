const DM_DATA_SOURCE_ALIAS: &str = "__fluxdb_source";

fn dm_load_data(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    filters: &[FilterSpec],
) -> fluxdb_core::Result<DataPage> {
    dm_load_data_with_expression(config, path, offset, limit, sort, filters, None, &[])
}

fn dm_load_data_digests(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    filters: &[FilterSpec],
    digest_columns: &[String],
) -> fluxdb_core::Result<DataPage> {
    dm_load_data_with_expression(
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

fn dm_load_data_expression(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    expression: &fluxdb_core::FilterExpression,
) -> fluxdb_core::Result<DataPage> {
    dm_load_data_with_expression(
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

fn dm_load_data_with_expression(
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
    let schema = dm_request_schema(config, path)?;
    let pagination = Pagination::new(offset, limit);
    let mut client = dm_connect(config)?;
    let columns = dm_load_columns(&mut client, &schema, &path.name)?;
    let table = dm_qualified_table(&schema, &path.name);
    let (where_sql, params) = match expression {
        Some(expression) => dm_expression_where_params(expression, &columns)?,
        None => dm_where_params(filters, &columns)?,
    };
    let order = dm_order_by_clause(sort, &columns, DM_DATA_SOURCE_ALIAS);
    // 多取一行只用于判断是否还有下一页，不把它展示给调用方。
    let fetch_limit = pagination.limit.saturating_add(1);
    let sql = format!(
        "SELECT {alias}.* FROM {table} {alias}{where_sql}{order} LIMIT ? OFFSET ?",
        alias = fluxdb_core::DamengDialect.quote_identifier(DM_DATA_SOURCE_ALIAS),
    );
    let mut bound = params.clone();
    bound.push(Param::Int(fetch_limit as i64));
    bound.push(Param::Int(pagination.offset as i64));
    let result = dm_execute(&mut client, &sql, &bound)?;
    client.close();
    let ExecuteResult::Query { rows, .. } = result else {
        return Err(Error::new(ErrorKind::Query, "达梦数据页返回类型异常"));
    };
    // 类型展示以表元数据为准：结果描述的 VARCHAR 精度常按字节给出，
    // 直接覆盖会把 VARCHAR(24) 显示成 VARCHAR(96)。
    let output_columns = columns;
    // 大字段按内容归属外置：同一张表的多次翻页共用一个归属，重新加载时整批替换。
    let owner = fluxdb_core::table_content_owner(path);
    dm_collect_page_with_digests(
        output_columns,
        rows.into_iter(),
        pagination,
        0,
        &owner,
        digest_columns,
    )
}

fn dm_order_by_clause(sort: &[SortSpec], columns: &[Column], alias: &str) -> String {
    let mut parts = Vec::new();
    for spec in sort {
        if columns.iter().any(|column| column.name == spec.field) {
            parts.push(format!(
                "{}.{} {}",
                fluxdb_core::DamengDialect.quote_identifier(alias),
                fluxdb_core::DamengDialect.quote_identifier(&spec.field),
                match spec.direction {
                    SortDirection::Asc => "ASC",
                    SortDirection::Desc => "DESC",
                }
            ));
        }
    }
    let sorted = sort.iter().map(|spec| spec.field.as_str()).collect::<Vec<_>>();
    for column in columns
        .iter()
        .filter(|column| column.primary_key && !sorted.contains(&column.name.as_str()))
    {
        parts.push(format!(
            "{}.{} ASC",
            fluxdb_core::DamengDialect.quote_identifier(alias),
            fluxdb_core::DamengDialect.quote_identifier(&column.name)
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {}", parts.join(", "))
    }
}

fn dm_where_params(
    filters: &[FilterSpec],
    columns: &[Column],
) -> fluxdb_core::Result<(String, Vec<Param>)> {
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    for filter in filters.iter().filter(|filter| filter.enabled) {
        if !data_filter_clause_is_pushable(filter) {
            return Err(Error::new(ErrorKind::Query, "达梦过滤条件缺少必要的比较值"));
        }
        let Some(column) = columns.iter().find(|column| column.name == filter.field) else {
            return Err(Error::new(
                ErrorKind::Query,
                format!("过滤引用了不存在的列: {}", filter.field),
            ));
        };
        dm_push_filter(&mut clauses, &mut params, filter, column);
    }
    if clauses.is_empty() {
        Ok((String::new(), params))
    } else {
        Ok((format!(" WHERE {}", clauses.join(" AND ")), params))
    }
}

fn dm_expression_where_params(
    expression: &fluxdb_core::FilterExpression,
    columns: &[Column],
) -> fluxdb_core::Result<(String, Vec<Param>)> {
    let mut params = Vec::new();
    let sql = dm_expression_sql(expression, columns, &mut params)?;
    if sql.trim().is_empty() {
        Ok((String::new(), params))
    } else {
        Ok((format!(" WHERE {sql}"), params))
    }
}

fn dm_expression_sql(
    expression: &fluxdb_core::FilterExpression,
    columns: &[Column],
    params: &mut Vec<Param>,
) -> fluxdb_core::Result<String> {
    use fluxdb_core::FilterExpression;
    Ok(match expression {
        FilterExpression::Condition(condition) => {
            let column = columns
                .iter()
                .find(|column| column.name == condition.field)
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::Query,
                        format!("过滤引用了不存在的列: {}", condition.field),
                    )
                })?;
            dm_expression_condition(condition, column, params)?
        }
        FilterExpression::And(items) => {
            let sql = items
                .iter()
                .map(|item| dm_expression_sql(item, columns, params))
                .collect::<fluxdb_core::Result<Vec<_>>>()?
                .into_iter()
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>();
            if sql.len() <= 1 {
                sql.into_iter().next().unwrap_or_default()
            } else {
                format!("({})", sql.join(" AND "))
            }
        }
        FilterExpression::Or(items) => {
            let sql = items
                .iter()
                .map(|item| dm_expression_sql(item, columns, params))
                .collect::<fluxdb_core::Result<Vec<_>>>()?
                .into_iter()
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>();
            if sql.len() <= 1 {
                sql.into_iter().next().unwrap_or_default()
            } else {
                format!("({})", sql.join(" OR "))
            }
        }
        FilterExpression::Not(inner) => {
            format!("NOT ({})", dm_expression_sql(inner, columns, params)?)
        }
        FilterExpression::Raw(raw) => raw.clone(),
    })
}

fn dm_expression_condition(
    condition: &fluxdb_core::FilterCondition,
    column: &Column,
    params: &mut Vec<Param>,
) -> fluxdb_core::Result<String> {
    use fluxdb_core::{FilterOperator, FilterValue};
    let name = fluxdb_core::DamengDialect.quote_identifier(&column.name);
    let bind = |params: &mut Vec<Param>, value: &FilterValue| {
        params.push(dm_filter_param(value));
        "?"
    };
    let first = || {
        condition
            .values
            .first()
            .ok_or_else(|| Error::new(ErrorKind::Query, "过滤条件缺少必要的比较值"))
    };
    Ok(match condition.op {
        FilterOperator::Eq => format!("{name} = {}", bind(params, first()?)),
        FilterOperator::Ne => format!("{name} != {}", bind(params, first()?)),
        FilterOperator::Lt => format!("{name} < {}", bind(params, first()?)),
        FilterOperator::Le => format!("{name} <= {}", bind(params, first()?)),
        FilterOperator::Gt => format!("{name} > {}", bind(params, first()?)),
        FilterOperator::Ge => format!("{name} >= {}", bind(params, first()?)),
        FilterOperator::IsNull => format!("{name} IS NULL"),
        FilterOperator::IsNotNull => format!("{name} IS NOT NULL"),
        FilterOperator::IsEmpty => format!("{name} = ''"),
        FilterOperator::IsNotEmpty => format!("{name} != ''"),
        FilterOperator::Contains
        | FilterOperator::NotContains
        | FilterOperator::StartsWith
        | FilterOperator::NotStartsWith
        | FilterOperator::EndsWith
        | FilterOperator::NotEndsWith => {
            let text = fluxdb_core::filter_value_text(first()?);
            let pattern = crate::filter_dialects::like_pattern(condition.op, &text)
                .ok_or_else(|| Error::new(ErrorKind::Query, "未知的 LIKE 运算符"))?;
            let negative = matches!(
                condition.op,
                FilterOperator::NotContains
                    | FilterOperator::NotStartsWith
                    | FilterOperator::NotEndsWith
            );
            let like_name = if crate::filter_dialects::filter_column_is_text(column) {
                name.clone()
            } else {
                format!("CAST({name} AS VARCHAR(4000))")
            };
            format!(
                "{like_name}{}LIKE {}{}",
                if negative { " NOT " } else { " " },
                bind(params, &FilterValue::Text(pattern)),
                if crate::filter_dialects::like_value_needs_escape(&text) {
                    crate::filter_dialects::LIKE_ESCAPE_CLAUSE
                } else {
                    ""
                }
            )
        }
        FilterOperator::Between | FilterOperator::NotBetween => {
            let start = condition.values.first().ok_or_else(|| {
                Error::new(ErrorKind::Query, "过滤条件缺少区间起点")
            })?;
            let end = condition.values.get(1).ok_or_else(|| {
                Error::new(ErrorKind::Query, "过滤条件缺少区间终点")
            })?;
            let first = bind(params, start);
            let second = bind(params, end);
            format!(
                "{name}{} BETWEEN {first} AND {second}",
                if condition.op == FilterOperator::NotBetween {
                    " NOT"
                } else {
                    ""
                }
            )
        }
        FilterOperator::InList | FilterOperator::NotInList => {
            if condition.values.is_empty() {
                return Err(Error::new(ErrorKind::Query, "过滤条件缺少列表值"));
            }
            let values = condition
                .values
                .iter()
                .map(|value| bind(params, value))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{name}{} IN ({values})",
                if condition.op == FilterOperator::NotInList {
                    " NOT"
                } else {
                    ""
                }
            )
        }
    })
}

fn dm_filter_param(value: &fluxdb_core::FilterValue) -> Param {
    use fluxdb_core::FilterValue;
    match value {
        FilterValue::Null => Param::Null,
        FilterValue::Text(value) => Param::Str(value.clone()),
        FilterValue::Integer(value) => Param::Int(*value),
        FilterValue::Float(value) => Param::Float(*value),
        FilterValue::Boolean(value) => Param::Bool(*value),
    }
}

fn dm_push_filter(
    clauses: &mut Vec<String>,
    params: &mut Vec<Param>,
    filter: &FilterSpec,
    column: &Column,
) {
    let name = format!(
        "{}.{}",
        fluxdb_core::DamengDialect.quote_identifier(DM_DATA_SOURCE_ALIAS),
        fluxdb_core::DamengDialect.quote_identifier(&column.name)
    );
    let mut push = |values: &[CellValue]| {
        for value in values {
            params.push(dm_param(value));
        }
    };
    match filter.op {
        FilterOp::IsNull | FilterOp::NotExists => clauses.push(format!("{name} IS NULL")),
        FilterOp::IsNotNull | FilterOp::Exists => clauses.push(format!("{name} IS NOT NULL")),
        FilterOp::IsEmpty => {
            push(&[CellValue::Text(String::new())]);
            clauses.push(format!("{name} = ?"));
        }
        FilterOp::IsNotEmpty => {
            push(&[CellValue::Text(String::new())]);
            clauses.push(format!("{name} != ?"));
        }
        FilterOp::Between | FilterOp::NotBetween => {
            push(&filter.values[..2]);
            clauses.push(format!(
                "{name}{} BETWEEN ? AND ?",
                if filter.op == FilterOp::NotBetween { " NOT" } else { "" }
            ));
        }
        FilterOp::InList | FilterOp::NotInList => {
            push(&filter.values);
            clauses.push(format!(
                "{name}{} IN ({})",
                if filter.op == FilterOp::NotInList { " NOT" } else { "" },
                std::iter::repeat_n("?", filter.values.len())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        FilterOp::Eq | FilterOp::NotEq => {
            push(&filter.values);
            let connector = if filter.op == FilterOp::Eq { " OR " } else { " AND " };
            clauses.push(
                filter
                    .values
                    .iter()
                    .map(|_| format!("{name} {} ?", if filter.op == FilterOp::Eq { "=" } else { "!=" }))
                    .collect::<Vec<_>>()
                    .join(connector),
            );
        }
        FilterOp::Contains
        | FilterOp::NotContains
        | FilterOp::StartsWith
        | FilterOp::NotStartsWith
        | FilterOp::EndsWith
        | FilterOp::NotEndsWith => {
            let value = filter.values.first().map(|value| match value {
                CellValue::Text(value) | CellValue::Json(value) => value.clone(),
                other => other.display_label(),
            });
            let Some(value) = value else { return };
            let (like, negate) = match filter.op {
                FilterOp::Contains | FilterOp::NotContains => (format!("%{value}%"), filter.op == FilterOp::NotContains),
                FilterOp::StartsWith | FilterOp::NotStartsWith => {
                    (format!("{value}%"), filter.op == FilterOp::NotStartsWith)
                }
                _ => (format!("%{value}"), filter.op == FilterOp::NotEndsWith),
            };
            params.push(Param::Str(like));
            clauses.push(format!("{name} {}LIKE ?", if negate { "NOT " } else { "" }));
        }
        FilterOp::GreaterThan
        | FilterOp::GreaterThanOrEqual
        | FilterOp::LessThan
        | FilterOp::LessThanOrEqual => {
            let Some(value) = filter.values.first() else { return };
            params.push(dm_param(value));
            let op = match filter.op {
                FilterOp::GreaterThan => ">",
                FilterOp::GreaterThanOrEqual => ">=",
                FilterOp::LessThan => "<",
                _ => "<=",
            };
            clauses.push(format!("{name} {op} ?"));
        }
    }
}

/// 按行身份读取二进制单元格完整值（同步写回大字段用）。
fn dm_load_cell_binary(
    config: &ConnectionConfig,
    path: &ObjectPath,
    identity: &fluxdb_core::RowIdentity,
    column: &str,
) -> fluxdb_core::Result<Vec<u8>> {
    match dm_load_cell_raw(config, path, identity, column)? {
        CellValue::Bytes(bytes) => Ok(bytes),
        CellValue::Text(text) => Ok(text.into_bytes()),
        CellValue::Null => Err(Error::new(ErrorKind::Query, "二进制单元格为 NULL")),
        _ => Err(Error::new(ErrorKind::Query, "二进制单元格类型异常")),
    }
}

/// 按行身份读取文本单元格完整值（同步写回大文本用）。
fn dm_load_cell_text(
    config: &ConnectionConfig,
    path: &ObjectPath,
    identity: &fluxdb_core::RowIdentity,
    column: &str,
) -> fluxdb_core::Result<String> {
    match dm_load_cell_raw(config, path, identity, column)? {
        CellValue::Text(text) | CellValue::Json(text) => Ok(text),
        CellValue::Bytes(bytes) => String::from_utf8(bytes)
            .map_err(|_| Error::new(ErrorKind::Query, "文本单元格不是合法 UTF-8")),
        CellValue::Null => Err(Error::new(ErrorKind::Query, "文本单元格为 NULL")),
        _ => Err(Error::new(ErrorKind::Query, "文本单元格类型异常")),
    }
}

/// 读取单列单行：达梦驱动直接把 LOB 读成 Str/Bytes，可直接取完整值。
fn dm_load_cell_raw(
    config: &ConnectionConfig,
    path: &ObjectPath,
    identity: &fluxdb_core::RowIdentity,
    column: &str,
) -> fluxdb_core::Result<CellValue> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(ErrorKind::Unsupported, "仅表和视图支持单元格读取"));
    }
    let schema = dm_request_schema(config, path)?;
    let mut client = dm_connect(config)?;
    let columns = dm_load_columns(&mut client, &schema, &path.name)?;
    if !columns
        .iter()
        .any(|candidate| candidate.name.eq_ignore_ascii_case(column))
    {
        return Err(Error::new(ErrorKind::Query, format!("字段不存在或已变更：{column}")));
    }
    let (where_sql, params) = dm_identity_where(identity, &columns)?;
    let sql = format!(
        "SELECT {} FROM {}{where_sql} LIMIT 1",
        fluxdb_core::DamengDialect.quote_identifier(column),
        dm_qualified_table(&schema, &path.name)
    );
    let result = dm_execute(&mut client, &sql, &params)?;
    client.close();
    let ExecuteResult::Query { rows, .. } = result else {
        return Err(Error::new(ErrorKind::Query, "单元格读取返回类型异常"));
    };
    let Some(value) = rows.into_iter().next().and_then(|row| row.into_iter().next()) else {
        return Err(Error::new(ErrorKind::Query, "未找到目标行"));
    };
    Ok(dm_cell_value(value))
}
