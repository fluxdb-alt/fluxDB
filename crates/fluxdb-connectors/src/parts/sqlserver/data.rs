fn sqlserver_load_data(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    filters: &[FilterSpec],
) -> fluxdb_core::Result<DataPage> {
    sqlserver_load_data_with_expression(config, path, offset, limit, sort, filters, None, &[])
}

fn sqlserver_load_data_digests(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    filters: &[FilterSpec],
    digest_columns: &[String],
) -> fluxdb_core::Result<DataPage> {
    sqlserver_load_data_with_expression(
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

fn sqlserver_load_data_expression(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    sort: &[SortSpec],
    expression: &fluxdb_core::FilterExpression,
) -> fluxdb_core::Result<DataPage> {
    sqlserver_load_data_with_expression(
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

fn sqlserver_load_data_with_expression(
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
    let schema = sqlserver_request_schema(config, path)?;
    let database = sqlserver_request_database(config, path.database.as_deref())?;
    let pagination = Pagination::new(offset, limit);
    let mut client = sqlserver_connect_database(config, Some(&database))?;
    let columns = sqlserver_load_columns(&mut client, &schema, &path.name)?;
    let query_columns = sqlserver_columns_to_page_columns(&columns);
    let table = sqlserver_qualified_table(&schema, &path.name);
    let mut parameters = Vec::new();
    let where_sql = match expression {
        Some(expression) => {
            sqlserver_expression_where_clause(expression, &query_columns, None, &mut parameters)?
        }
        None => sqlserver_where_clause(filters, &query_columns, None, &mut parameters)?,
    };
    let order = sqlserver_data_order_by(sort, &query_columns, None);
    // 二进制列按摘要投影：正文永不进结果页，与 MySQL/PG 同一口径。
    let (select_list, outer_list, positions) =
        sqlserver_page_select_list(&query_columns, digest_columns);
    let fetch_limit = pagination.limit.saturating_add(1);
    let sql = if pagination.offset == 0 {
        parameters.push(fluxdb_core::SqlParameter::I64(fetch_limit.to_string()));
        let top = sqlserver_placeholder(parameters.len());
        format!("SELECT TOP ({top}) {select_list} FROM {table}{where_sql}{order}")
    } else {
        parameters.push(fluxdb_core::SqlParameter::I64(pagination.offset.to_string()));
        let skip = sqlserver_placeholder(parameters.len());
        parameters.push(fluxdb_core::SqlParameter::I64(fetch_limit.to_string()));
        let fetch = sqlserver_placeholder(parameters.len());
        let order_by = if order.is_empty() {
            " ORDER BY (SELECT NULL)".to_string()
        } else {
            order.clone()
        };
        // 用 ROW_NUMBER 而不是 OFFSET/FETCH：后者要求 SQL Server 2012+，
        // 且无排序时必须额外构造 ORDER BY。ROW_NUMBER 在 2005+ 均可工作。
        // `order_by` 自带 `ORDER BY` 前缀，窗口函数内必须保留。
        // 外层只能引用子查询的**输出**列名（二进制已折成 3 个别名），
        // 不能再套一次投影表达式——子查询里没有原始二进制列。
        let order_by = order_by.trim_start();
        format!(
            "SELECT {outer_list} FROM (SELECT {select_list}, ROW_NUMBER() OVER ({order_by}) AS [__fluxdb_row_num] FROM {table}{where_sql}) AS [__fluxdb_page] WHERE [__fluxdb_row_num] > {skip} AND [__fluxdb_row_num] <= {skip} + {fetch} ORDER BY [__fluxdb_row_num]"
        )
    };
    // 大字段按内容归属外置：同一张表的多次翻页共用一个归属，重新加载时整批替换。
    let owner = fluxdb_core::table_content_owner(path);
    let batch = sqlserver_block_on(async {
        let batch = sqlserver_run_query_with(&mut client, &sql, &parameters, Some(&owner)).await?;
        sqlserver_close(client).await;
        Ok(batch)
    })?;
    let mut rows = batch
        .result_sets
        .into_iter()
        .next()
        .ok_or_else(|| Error::new(ErrorKind::Query, "SQL Server 数据页返回类型异常"))?
        .rows
        .into_iter()
        .map(|row| {
            sqlserver_project_row(row.values, &query_columns, &positions, digest_columns)
        })
        .collect::<Vec<_>>();
    let has_more = rows.len() > pagination.limit as usize;
    rows.truncate(pagination.limit as usize);
    Ok(DataPage {
        columns: query_columns,
        rows,
        offset: pagination.offset,
        limit: pagination.limit,
        has_more,
        incomplete: None,
    })
}

/// 显式列清单：`sql_variant` 在 tiberius 0.12.3 的列元数据解码中是未实现分支
/// （`todo!()`），直接 `SELECT *` 会让整个桌面进程 abort。这里把 `sql_variant`
/// 转成 `nvarchar(max)`，既避免驱动崩溃，也让用户能看到可读值。
fn sqlserver_data_select_list(columns: &[Column]) -> String {
    columns
        .iter()
        .map(|column| {
            let quoted = fluxdb_core::SqlServerDialect.quote_identifier(&column.name);
            if sqlserver_is_sql_variant(column) {
                format!("CAST({quoted} AS nvarchar(max)) AS {quoted}")
            } else {
                quoted
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn sqlserver_is_sql_variant(column: &Column) -> bool {
    column.type_name.as_deref().is_some_and(|type_name| {
        type_name
            .trim()
            .split(['(', ' '])
            .next()
            .is_some_and(|base| base.eq_ignore_ascii_case("sql_variant"))
    })
}

/// 数据页专用列清单：二进制列按摘要投影（3 个输出列），其余列保持原值。
///
/// 不能复用 [`sqlserver_data_select_list`]：那是给导出预览生成「真实值」SQL 用的，
/// 把摘要别名写进去等于导出一堆长度数字。返回的位置表描述每个真实列占用的
/// 投影下标，供 [`sqlserver_project_row`] 折回一行。
///
/// 同时返回外层列清单：翻页 SQL 用 `ROW_NUMBER` 包了一层子查询，外层只能引用
/// 子查询的输出列名，不能再套一次投影表达式（子查询里没有原始二进制列）。
fn sqlserver_page_select_list(
    columns: &[Column],
    digest_columns: &[String],
) -> (String, String, Vec<(Vec<usize>, usize)>) {
    let mut inner = Vec::with_capacity(columns.len());
    let mut outer = Vec::with_capacity(columns.len());
    let mut positions = Vec::with_capacity(columns.len());
    let mut position = 0usize;
    for (index, column) in columns.iter().enumerate() {
        if digest_columns
            .iter()
            .any(|digest| digest.eq_ignore_ascii_case(&column.name))
        {
            // 摘要列：单列输出小写十六进制哈希。先 CAST 成 varbinary 以兼容
            // text/ntext/image 等 HASHBYTES 不接受的历史类型。
            let name = fluxdb_core::SqlServerDialect.quote_identifier(&column.name);
            positions.push((vec![position], index));
            position += 1;
            inner.push(format!(
                "LOWER(CONVERT(varchar(64), HASHBYTES('SHA2_256', CAST({name} AS varbinary(max))), 2)) AS {name}"
            ));
            outer.push(name);
            continue;
        }
        let width = if is_binary_column(column) { 3 } else { 1 };
        positions.push(((position..position + width).collect(), index));
        position += width;
        inner.push(sqlserver_select_expr(column));
        if is_binary_column(column) {
            outer.extend(
                ["is_null", "byte_length", "preview_hex"].map(|suffix| {
                    fluxdb_core::SqlServerDialect
                        .quote_identifier(&binary_alias(&column.name, suffix))
                }),
            );
        } else {
            outer.push(fluxdb_core::SqlServerDialect.quote_identifier(&column.name));
        }
    }
    (inner.join(", "), outer.join(", "), positions)
}

/// 把投影行折回真实列：二进制三列合成 `BinarySummary`，其余列原样取出。
fn sqlserver_project_row(
    values: Vec<CellValue>,
    columns: &[Column],
    positions: &[(Vec<usize>, usize)],
    digest_columns: &[String],
) -> Row {
    Row {
        values: positions
            .iter()
            .map(|(indexes, column_index)| {
                let column = &columns[*column_index];
                if digest_columns
                    .iter()
                    .any(|digest| digest.eq_ignore_ascii_case(&column.name))
                {
                    // 摘要列已是文本，直接取第一个投影下标。
                    return indexes
                        .first()
                        .and_then(|index| values.get(*index))
                        .cloned()
                        .unwrap_or(CellValue::Null);
                }
                if is_binary_column(column) {
                    let is_null = indexes
                        .first()
                        .and_then(|index| values.get(*index))
                        .and_then(cell_i64)
                        .is_some_and(|value| value != 0);
                    let byte_length = indexes
                        .get(1)
                        .and_then(|index| values.get(*index))
                        .and_then(cell_i64)
                        .unwrap_or(0)
                        .max(0) as u64;
                    let preview_hex = indexes
                        .get(2)
                        .and_then(|index| values.get(*index))
                        .and_then(cell_text)
                        .filter(|value| !value.is_empty());
                    return CellValue::BinarySummary(binary_summary(
                        column,
                        is_null,
                        byte_length,
                        preview_hex,
                    ));
                }
                indexes
                    .first()
                    .and_then(|index| values.get(*index))
                    .cloned()
                    .unwrap_or(CellValue::Null)
            })
            .collect(),
    }
}

fn sqlserver_data_order_by(
    sort: &[SortSpec],
    columns: &[Column],
    alias: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    for spec in sort {
        if columns.iter().any(|column| column.name == spec.field) {
            parts.push(format!(
                "{} {}",
                sqlserver_column_reference(&spec.field, alias),
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
            "{} ASC",
            sqlserver_column_reference(&column.name, alias)
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {}", parts.join(", "))
    }
}

/// 导出预览：统计满足过滤条件的行数，并生成与实际导出一致的 SELECT。
///
/// 这里不复用 `load_data`，因为预览只需要计数和 SQL 文本，不应把整页数据
/// 读进内存；过滤参数仍走真实绑定，避免把用户值拼进 SQL。
fn sqlserver_preview_data_export(
    config: &ConnectionConfig,
    path: &ObjectPath,
    fields: &[String],
    sort: &[SortSpec],
    filters: &[FilterSpec],
) -> fluxdb_core::Result<DataExportPreview> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "仅表和视图支持导出预览",
        ));
    }
    let schema = sqlserver_request_schema(config, path)?;
    let database = sqlserver_request_database(config, path.database.as_deref())?;
    let mut client = sqlserver_connect_database(config, Some(&database))?;
    let columns = sqlserver_columns_to_page_columns(&sqlserver_load_columns(
        &mut client,
        &schema,
        &path.name,
    )?);
    let select_list = sqlserver_data_select_list(&columns);
    let table = sqlserver_qualified_table(&schema, &path.name);
    let mut parameters = Vec::new();
    let where_sql = sqlserver_where_clause(filters, &columns, None, &mut parameters)?;
    let count_sql = format!("SELECT COUNT_BIG(*) FROM {table}{where_sql}");
    let batch = sqlserver_block_on(async {
        let batch = sqlserver_run_query(&mut client, &count_sql, &parameters).await?;
        sqlserver_close(client).await;
        Ok(batch)
    })?;
    let row_count = sqlserver_first_rows(&batch, "SQL Server 导出行数返回类型异常")?
        .first()
        .and_then(|row| row.values.first())
        .and_then(cell_i64)
        .unwrap_or(0)
        .max(0) as u64;
    Ok(DataExportPreview {
        sql: sqlserver_export_preview_sql(&table, fields, sort, filters, &columns, &select_list),
        row_count,
    })
}

/// 生成导出 SQL（与实际导出一致：显式字段、过滤、排序）。
///
/// 不追加 TOP：导出是完整结果集；行数上限由导出执行器分页控制。
fn sqlserver_export_preview_sql(
    table: &str,
    fields: &[String],
    sort: &[SortSpec],
    filters: &[FilterSpec],
    columns: &[Column],
    data_select_list: &str,
) -> String {
    let selected = fields
        .iter()
        .filter_map(|field| columns.iter().find(|column| column.name == *field))
        .map(|column| fluxdb_core::SqlServerDialect.quote_identifier(&column.name))
        .collect::<Vec<_>>();
    let select_list = if selected.is_empty() {
        data_select_list.to_string()
    } else {
        selected.join(", ")
    };
    // 预览 SQL 给用户看，不绑定参数；过滤值直接渲染成安全字面量。
    let where_sql = sqlserver_preview_where_clause(filters, columns);
    let order = sqlserver_data_order_by(sort, columns, None);
    format!("SELECT {select_list}\nFROM {table}{where_sql}{order}")
}

/// 生成供展示的 WHERE 子句。只接受可下推的过滤条件；标识符来自已加载列，
/// 值全部按类型渲染成字面量（字符串用 `N''` 并转义单引号）。
fn sqlserver_preview_where_clause(
    filters: &[FilterSpec],
    columns: &[Column],
) -> String {
    let mut clauses = Vec::new();
    for filter in filters.iter().filter(|filter| filter.enabled) {
        if !data_filter_clause_is_pushable(filter) {
            continue;
        }
        let Some(column) = columns.iter().find(|column| column.name == filter.field) else {
            continue;
        };
        let name = fluxdb_core::SqlServerDialect.quote_identifier(&column.name);
        let literal = |value: &CellValue| format!(
            "{}",
            sqlserver_preview_value_literal(value)
        );
        let clause = match filter.op {
            FilterOp::IsNull | FilterOp::NotExists => format!("{name} IS NULL"),
            FilterOp::IsNotNull | FilterOp::Exists => format!("{name} IS NOT NULL"),
            FilterOp::IsEmpty => format!("{name} = N''"),
            FilterOp::IsNotEmpty => format!("{name} <> N''"),
            FilterOp::Between | FilterOp::NotBetween => {
                let Some(start) = filter.values.first() else {
                    continue;
                };
                let Some(end) = filter.values.get(1) else {
                    continue;
                };
                format!(
                    "{name}{} BETWEEN {} AND {}",
                    if filter.op == FilterOp::NotBetween {
                        " NOT"
                    } else {
                        ""
                    },
                    literal(start),
                    literal(end)
                )
            }
            FilterOp::InList | FilterOp::NotInList => {
                if filter.values.is_empty() {
                    continue;
                }
                format!(
                    "{name}{} IN ({})",
                    if filter.op == FilterOp::NotInList {
                        " NOT"
                    } else {
                        ""
                    },
                    filter
                        .values
                        .iter()
                        .map(literal)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            FilterOp::Eq | FilterOp::NotEq => {
                if filter.values.is_empty() {
                    continue;
                }
                let op = if filter.op == FilterOp::Eq { "=" } else { "<>" };
                let joiner = if filter.op == FilterOp::Eq { " OR " } else { " AND " };
                let predicates = filter
                    .values
                    .iter()
                    .map(|value| {
                        if matches!(value, CellValue::Null) {
                            if filter.op == FilterOp::Eq {
                                format!("{name} IS NULL")
                            } else {
                                format!("{name} IS NOT NULL")
                            }
                        } else {
                            format!("{name} {op} {}", literal(value))
                        }
                    })
                    .collect::<Vec<_>>();
                if predicates.len() > 1 {
                    format!("({})", predicates.join(joiner))
                } else {
                    predicates.join(joiner)
                }
            }
            FilterOp::Contains
            | FilterOp::NotContains
            | FilterOp::StartsWith
            | FilterOp::NotStartsWith
            | FilterOp::EndsWith
            | FilterOp::NotEndsWith => {
                let Some(value) = filter.values.first() else {
                    continue;
                };
                let text = match value {
                    CellValue::Text(value) | CellValue::Json(value) => value.clone(),
                    other => other.display_label(),
                };
                let pattern = match filter.op {
                    FilterOp::Contains | FilterOp::NotContains => format!("%{text}%"),
                    FilterOp::StartsWith | FilterOp::NotStartsWith => format!("{text}%"),
                    _ => format!("%{text}"),
                };
                let negative = matches!(
                    filter.op,
                    FilterOp::NotContains | FilterOp::NotStartsWith | FilterOp::NotEndsWith
                );
                format!(
                    "{name} {}LIKE N'{}'",
                    if negative { "NOT " } else { "" },
                    pattern.replace('\'', "''")
                )
            }
            FilterOp::GreaterThan
            | FilterOp::GreaterThanOrEqual
            | FilterOp::LessThan
            | FilterOp::LessThanOrEqual => {
                let Some(value) = filter.values.first() else {
                    continue;
                };
                let op = match filter.op {
                    FilterOp::GreaterThan => ">",
                    FilterOp::GreaterThanOrEqual => ">=",
                    FilterOp::LessThan => "<",
                    _ => "<=",
                };
                format!("{name} {op} {}", literal(value))
            }
        };
        clauses.push(clause);
    }
    if clauses.is_empty() {
        String::new()
    } else {
        format!("\nWHERE {}", clauses.join(" AND "))
    }
}

fn sqlserver_preview_value_literal(value: &CellValue) -> String {
    match value {
        CellValue::Null => "NULL".into(),
        CellValue::Bool(value) => if *value { "1" } else { "0" }.into(),
        CellValue::I64(value) => value.to_string(),
        CellValue::F64(value) if value.is_finite() => value.to_string(),
        CellValue::F64(_) => "NULL".into(),
        CellValue::Text(value) | CellValue::Json(value) => {
            format!("N'{}'", value.replace('\'', "''"))
        }
        // 预览 SQL 里不能出现半截值：写 NULL 并用注释说明，避免用户把它当可执行导出。
        CellValue::TruncatedText { total_chars, .. } => {
            format!("NULL /* 截断的 CLOB：共 {total_chars} 字符，需完整读取 */")
        }
        // 同理：大字段只有预览，不能当可执行导出。
        CellValue::LargeValue(large) => match large.byte_length {
            Some(length) => format!("NULL /* 超大字段：共 {length} 字节，需按内容引用读取 */"),
            None => "NULL /* 超大字段：完整大小未知，需按内容引用读取 */".to_string(),
        },
        CellValue::Bytes(value) => format!(
            "0x{}",
            value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
        CellValue::BinarySummary(summary) => summary
            .preview_hex
            .as_deref()
            .filter(|_| !summary.is_null)
            .map(|hex| format!("0x{hex}"))
            .unwrap_or_else(|| "NULL".into()),
    }
}

fn sqlserver_column_reference(column: &str, alias: Option<&str>) -> String {
    match alias {
        Some(alias) => format!(
            "{}.{}",
            fluxdb_core::SqlServerDialect.quote_identifier(alias),
            fluxdb_core::SqlServerDialect.quote_identifier(column)
        ),
        None => fluxdb_core::SqlServerDialect.quote_identifier(column),
    }
}

#[cfg(test)]
mod sqlserver_page_projection_tests {
    use super::*;

    fn columns() -> Vec<Column> {
        vec![
            Column {
                name: "id".into(),
                type_name: Some("int".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            Column {
                name: "photo".into(),
                type_name: Some("varbinary(max)".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
        ]
    }

    /// 翻页 SQL 的外层不能重复投影表达式：子查询里二进制列已经折成 3 个别名。
    #[test]
    fn outer_select_uses_projected_aliases() {
        let (inner, outer, positions) = sqlserver_page_select_list(&columns(), &[]);
        assert!(inner.contains("[photo__byte_length]"), "内层必须投影字节数：{inner}");
        assert!(!outer.contains("DATALENGTH"), "外层不能再用原始列计算：{outer}");
        assert_eq!(
            outer,
            "[id], [photo__is_null], [photo__byte_length], [photo__preview_hex]"
        );
        assert_eq!(
            positions,
            vec![(vec![0], 0), (vec![1, 2, 3], 1)],
            "位置表要让 3 个投影列折回 1 个真实列"
        );
    }

    #[test]
    fn digest_column_projects_single_hash_value() {
        let columns = columns();
        let (inner, outer, positions) =
            sqlserver_page_select_list(&columns, &["photo".into()]);
        assert_eq!(positions[1], (vec![1], 1));
        assert!(inner.contains("HASHBYTES('SHA2_256'"), "{inner}");
        assert_eq!(outer.split(", ").count(), 2, "{outer}");
    }

    #[test]
    fn projected_row_folds_binary_summary() {
        let columns = columns();
        let (_, _, positions) = sqlserver_page_select_list(&columns, &[]);
        let row = sqlserver_project_row(
            vec![
                CellValue::I64(7),
                CellValue::I64(0),
                CellValue::I64(300),
                CellValue::Text("0AFF".into()),
            ],
            &columns,
            &positions,
            &[],
        );
        assert_eq!(row.values[0], CellValue::I64(7));
        assert_eq!(
            row.values[1],
            CellValue::BinarySummary(fluxdb_core::BinaryCellSummary {
                type_name: "varbinary(max)".into(),
                is_null: false,
                byte_length: 300,
                preview_hex: Some("0AFF".into()),
            })
        );
    }

    /// 空二进制不能显示成「有值但前缀为空」：`preview_hex` 必须是 `None`。
    #[test]
    fn empty_binary_has_no_preview() {
        let columns = columns();
        let (_, _, positions) = sqlserver_page_select_list(&columns, &[]);
        let row = sqlserver_project_row(
            vec![
                CellValue::I64(7),
                CellValue::I64(0),
                CellValue::I64(0),
                CellValue::Text(String::new()),
            ],
            &columns,
            &positions,
            &[],
        );
        assert!(matches!(
            &row.values[1],
            CellValue::BinarySummary(summary) if summary.preview_hex.is_none() && summary.byte_length == 0
        ));
    }
}

/// 按行身份读取二进制单元格完整值（同步写回大字段用）。
///
/// 数据页的二进制列只有摘要；写回必须重新取一次完整值，不能拿摘要凑数。
/// 统一 CAST 成 `varbinary(max)` 以兼容 `image` 等历史类型。
fn sqlserver_load_cell_binary(
    config: &ConnectionConfig,
    path: &ObjectPath,
    identity: &fluxdb_core::RowIdentity,
    column: &str,
) -> fluxdb_core::Result<Vec<u8>> {
    let cell = sqlserver_load_cell_raw(config, path, identity, column, "varbinary(max)")?;
    match cell {
        CellValue::Bytes(bytes) => Ok(bytes),
        CellValue::Text(text) => Ok(text.into_bytes()),
        CellValue::Null => Err(Error::new(ErrorKind::Query, "二进制单元格为 NULL")),
        _ => Err(Error::new(ErrorKind::Query, "二进制单元格类型异常")),
    }
}

/// 按行身份读取文本单元格完整值（同步写回大文本用）。
fn sqlserver_load_cell_text(
    config: &ConnectionConfig,
    path: &ObjectPath,
    identity: &fluxdb_core::RowIdentity,
    column: &str,
) -> fluxdb_core::Result<String> {
    let cell = sqlserver_load_cell_raw(config, path, identity, column, "nvarchar(max)")?;
    match cell {
        CellValue::Text(text) | CellValue::Json(text) => Ok(text),
        CellValue::Bytes(bytes) => String::from_utf8(bytes)
            .map_err(|_| Error::new(ErrorKind::Query, "文本单元格不是合法 UTF-8")),
        CellValue::Null => Err(Error::new(ErrorKind::Query, "文本单元格为 NULL")),
        _ => Err(Error::new(ErrorKind::Query, "文本单元格类型异常")),
    }
}

/// 读取单列单行并按 `cast_type` 转成驱动可解码的类型。
fn sqlserver_load_cell_raw(
    config: &ConnectionConfig,
    path: &ObjectPath,
    identity: &fluxdb_core::RowIdentity,
    column: &str,
    cast_type: &str,
) -> fluxdb_core::Result<CellValue> {
    if !matches!(path.kind, ObjectKind::Table | ObjectKind::View) {
        return Err(Error::new(ErrorKind::Unsupported, "仅表和视图支持单元格读取"));
    }
    let schema = sqlserver_request_schema(config, path)?;
    let database = sqlserver_request_database(config, path.database.as_deref())?;
    let mut client = sqlserver_connect_database(config, Some(&database))?;
    let columns = sqlserver_load_columns(&mut client, &schema, &path.name)?;
    if !columns
        .iter()
        .any(|candidate| candidate.name.eq_ignore_ascii_case(column))
    {
        return Err(Error::new(ErrorKind::Query, format!("字段不存在或已变更：{column}")));
    }
    let (where_sql, values) = sqlserver_identity_predicate(identity, &columns)?;
    let qualified = fluxdb_core::SqlServerDialect.quote_identifier(column);
    let sql = format!(
        "SELECT CAST({qualified} AS {cast_type}) FROM {}{where_sql}",
        sqlserver_qualified_table(&schema, &path.name)
    );
    let batch = sqlserver_block_on(sqlserver_run_query_bound(&mut client, &sql, &values))?;
    sqlserver_block_on(sqlserver_close(client));
    let row = batch
        .result_sets
        .first()
        .and_then(|set| set.rows.first())
        .ok_or_else(|| Error::new(ErrorKind::Query, "未找到目标行"))?;
    row.values
        .first()
        .cloned()
        .ok_or_else(|| Error::new(ErrorKind::Query, "未读取到目标列"))
}
