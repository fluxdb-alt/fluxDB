#[derive(Clone, Debug)]
struct SqlServerColumnInfo {
    name: String,
    ordinal: u32,
    data_type: String,
    nullable: bool,
    primary_key: bool,
    identity: bool,
    computed: bool,
    generated: bool,
    default_expr: Option<String>,
    comment: Option<String>,
}

fn sqlserver_connect_database(
    config: &ConnectionConfig,
    database: Option<&str>,
) -> fluxdb_core::Result<SqlServerClient> {
    let mut profile = sqlserver_profile(config)?;
    if let Some(database) = database.map(str::trim).filter(|database| !database.is_empty()) {
        profile.basic.database = database.to_string();
    }
    sqlserver_connect_profile(profile)
}

fn sqlserver_load_columns(
    client: &mut SqlServerClient,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<SqlServerColumnInfo>> {
    let batch = sqlserver_block_on(sqlserver_run_query(
        client,
        "SELECT c.name, c.column_id, ty.name AS type_name, c.max_length, c.precision, c.scale, \
                c.is_nullable, c.is_identity, c.is_computed, \
                CASE WHEN cc.definition IS NOT NULL THEN 1 ELSE 0 END AS is_generated, \
                dc.definition AS default_expr, \
                CAST(ep.value AS nvarchar(max)) AS comment, \
                CASE WHEN pk.column_id IS NULL THEN 0 ELSE 1 END AS is_primary_key \
         FROM sys.objects o \
         JOIN sys.schemas s ON s.schema_id = o.schema_id \
         JOIN sys.columns c ON c.object_id = o.object_id \
         JOIN sys.types ty ON ty.user_type_id = c.user_type_id \
         LEFT JOIN sys.default_constraints dc ON dc.object_id = c.default_object_id \
         LEFT JOIN sys.computed_columns cc ON cc.object_id = c.object_id AND cc.column_id = c.column_id \
         LEFT JOIN sys.extended_properties ep \
           ON ep.class = 1 AND ep.major_id = c.object_id AND ep.minor_id = c.column_id \
          AND ep.name = 'MS_Description' \
         LEFT JOIN ( \
             SELECT ic.object_id, ic.column_id \
             FROM sys.indexes i \
             JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
             WHERE i.is_primary_key = 1 \
         ) pk ON pk.object_id = c.object_id AND pk.column_id = c.column_id \
         WHERE s.name = @P1 AND o.name = @P2 \
         ORDER BY c.column_id",
        &[
            fluxdb_core::SqlParameter::Text(schema.to_string()),
            fluxdb_core::SqlParameter::Text(table.to_string()),
        ],
    ))?;
    let rows = sqlserver_first_rows(&batch, "SQL Server 列元数据返回类型异常")?;
    rows.into_iter()
        .map(|row| {
            let name = row
                .values
                .first()
                .and_then(cell_text)
                .ok_or_else(|| Error::new(ErrorKind::Query, "SQL Server 列元数据缺少字段名"))?;
            let type_name = row.values.get(2).and_then(cell_text).unwrap_or_default();
            let max_length = row.values.get(3).and_then(cell_i64).unwrap_or(-1);
            let precision = row.values.get(4).and_then(cell_i64).unwrap_or(0);
            let scale = row.values.get(5).and_then(cell_i64).unwrap_or(0);
            Ok(SqlServerColumnInfo {
                name,
                ordinal: row.values.get(1).and_then(cell_i64).and_then(|value| u32::try_from(value).ok()).unwrap_or(0),
                data_type: sqlserver_type_display(&type_name, max_length, precision, scale),
                nullable: row.values.get(6).and_then(cell_bool).unwrap_or(true),
                identity: row.values.get(7).and_then(cell_bool).unwrap_or(false),
                computed: row.values.get(8).and_then(cell_bool).unwrap_or(false),
                generated: row.values.get(9).and_then(cell_bool).unwrap_or(false),
                default_expr: row.values.get(10).and_then(cell_text),
                comment: row.values.get(11).and_then(cell_text),
                primary_key: row.values.get(12).and_then(cell_bool).unwrap_or(false),
            })
        })
        .collect()
}

fn sqlserver_type_display(type_name: &str, max_length: i64, precision: i64, scale: i64) -> String {
    let lower = type_name.to_ascii_lowercase();
    match lower.as_str() {
        "varchar" | "char" | "varbinary" | "binary" => {
            if max_length < 0 {
                format!("{lower}(max)")
            } else {
                format!("{lower}({max_length})")
            }
        }
        "nvarchar" | "nchar" => {
            if max_length < 0 {
                format!("{lower}(max)")
            } else {
                format!("{lower}({})", (max_length / 2).max(0))
            }
        }
        "decimal" | "numeric" => format!("{lower}({precision},{scale})"),
        "datetime2" | "time" | "datetimeoffset" => format!("{lower}({scale})"),
        _ => type_name.to_string(),
    }
}

fn cell_bool(value: &CellValue) -> Option<bool> {
    match value {
        CellValue::Bool(value) => Some(*value),
        CellValue::I64(value) => Some(*value != 0),
        CellValue::Text(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn sqlserver_columns_to_page_columns(columns: &[SqlServerColumnInfo]) -> Vec<Column> {
    columns
        .iter()
        .map(|column| Column {
            name: column.name.clone(),
            type_name: Some(column.data_type.clone()),
            nullable: column.nullable,
            primary_key: column.primary_key,
            comment: column.comment.clone(),
        })
        .collect()
}

fn sqlserver_completion_columns(
    path: &ObjectPath,
    columns: Vec<SqlServerColumnInfo>,
) -> Vec<CompletionColumn> {
    columns
        .into_iter()
        .map(|column| CompletionColumn {
            database: path.database.clone(),
            schema: path.schema.clone(),
            table: path.name.clone(),
            name: column.name,
            type_name: Some(column.data_type),
            nullable: column.nullable,
            primary_key: column.primary_key,
            comment: column.comment,
            stable: Some(u64::from(column.ordinal)),
        })
        .collect()
}

fn sqlserver_indexes_for(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<IndexInfo>> {
    let schema = sqlserver_request_schema(config, path)?;
    let database = sqlserver_request_database(config, path.database.as_deref())?;
    let mut client = sqlserver_connect_database(config, Some(&database))?;
    let indexes = sqlserver_load_indexes(&mut client, &schema, &path.name)?;
    sqlserver_block_on(sqlserver_close(client));
    Ok(indexes)
}

fn sqlserver_load_indexes(
    client: &mut SqlServerClient,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<IndexInfo>> {
    let batch = sqlserver_block_on(sqlserver_run_query(
        client,
        "SELECT i.name, i.is_unique, i.is_primary_key, i.type_desc, c.name, ic.key_ordinal, ic.is_descending_key \
         FROM sys.objects o \
         JOIN sys.schemas s ON s.schema_id = o.schema_id \
         JOIN sys.indexes i ON i.object_id = o.object_id \
         JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id AND ic.is_included_column = 0 \
         JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id \
         WHERE s.name = @P1 AND o.name = @P2 AND i.name IS NOT NULL \
         ORDER BY i.index_id, ic.key_ordinal",
        &[
            fluxdb_core::SqlParameter::Text(schema.to_string()),
            fluxdb_core::SqlParameter::Text(table.to_string()),
        ],
    ))?;
    let rows = sqlserver_first_rows(&batch, "SQL Server 索引元数据返回类型异常")?;
    let mut indexes: Vec<IndexInfo> = Vec::new();
    for row in rows {
        let Some(name) = row.values.first().and_then(cell_text) else {
            continue;
        };
        let Some(column) = row.values.get(4).and_then(cell_text) else {
            continue;
        };
        if let Some(index) = indexes.iter_mut().find(|index| index.name == name) {
            index.columns.push(column);
            continue;
        }
        indexes.push(IndexInfo {
            name,
            columns: vec![column],
            is_unique: row.values.get(1).and_then(cell_bool).unwrap_or(false),
            is_primary: row.values.get(2).and_then(cell_bool).unwrap_or(false),
            index_type: row.values.get(3).and_then(cell_text),
            comment: None,
        });
    }
    Ok(indexes)
}

fn sqlserver_foreign_keys_for(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
    let schema = sqlserver_request_schema(config, path)?;
    let database = sqlserver_request_database(config, path.database.as_deref())?;
    let mut client = sqlserver_connect_database(config, Some(&database))?;
    let foreign_keys = sqlserver_load_foreign_keys(&mut client, &schema, &path.name)?;
    sqlserver_block_on(sqlserver_close(client));
    Ok(foreign_keys)
}

fn sqlserver_load_foreign_keys(
    client: &mut SqlServerClient,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
    let batch = sqlserver_block_on(sqlserver_run_query(
        client,
        "SELECT fk.name, rs.name AS ref_schema, rt.name AS ref_table, \
                rc.name AS ref_column, pc.name AS column_name, fkc.constraint_column_id \
         FROM sys.foreign_keys fk \
         JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id \
         JOIN sys.tables pt ON pt.object_id = fk.parent_object_id \
         JOIN sys.schemas ps ON ps.schema_id = pt.schema_id \
         JOIN sys.columns pc ON pc.object_id = pt.object_id AND pc.column_id = fkc.parent_column_id \
         JOIN sys.tables rt ON rt.object_id = fk.referenced_object_id \
         JOIN sys.schemas rs ON rs.schema_id = rt.schema_id \
         JOIN sys.columns rc ON rc.object_id = rt.object_id AND rc.column_id = fkc.referenced_column_id \
         WHERE ps.name = @P1 AND pt.name = @P2 \
         ORDER BY fk.name, fkc.constraint_column_id",
        &[
            fluxdb_core::SqlParameter::Text(schema.to_string()),
            fluxdb_core::SqlParameter::Text(table.to_string()),
        ],
    ))?;
    let rows = sqlserver_first_rows(&batch, "SQL Server 外键元数据返回类型异常")?;
    let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
    for row in rows {
        let Some(name) = row.values.first().and_then(cell_text) else {
            continue;
        };
        let ref_schema = row.values.get(1).and_then(cell_text);
        let ref_table = row.values.get(2).and_then(cell_text).unwrap_or_default();
        let ref_column = row.values.get(3).and_then(cell_text).unwrap_or_default();
        let column = row.values.get(4).and_then(cell_text).unwrap_or_default();
        if let Some(fk) = foreign_keys.iter_mut().find(|fk| fk.name == name) {
            fk.columns.push(column);
            fk.ref_columns.push(ref_column);
            continue;
        }
        foreign_keys.push(ForeignKeyInfo {
            name,
            column: column.clone(),
            ref_schema,
            ref_table,
            ref_column: ref_column.clone(),
            columns: vec![column],
            ref_columns: vec![ref_column],
        });
    }
    Ok(foreign_keys)
}

fn sqlserver_triggers_for(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<TriggerInfo>> {
    let schema = sqlserver_request_schema(config, path)?;
    let database = sqlserver_request_database(config, path.database.as_deref())?;
    let mut client = sqlserver_connect_database(config, Some(&database))?;
    let triggers = sqlserver_load_triggers(&mut client, &schema, &path.name)?;
    sqlserver_block_on(sqlserver_close(client));
    Ok(triggers)
}

fn sqlserver_load_triggers(
    client: &mut SqlServerClient,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<TriggerInfo>> {
    let batch = sqlserver_block_on(sqlserver_run_query(
        client,
        "SELECT tr.name, \
                STUFF((SELECT ',' + te.type_desc \
                       FROM sys.trigger_events te \
                       WHERE te.object_id = tr.object_id \
                       ORDER BY te.type_desc \
                       FOR XML PATH(''), TYPE).value('.', 'nvarchar(max)'), 1, 1, '') AS events, \
                CASE WHEN tr.is_instead_of_trigger = 1 THEN 'INSTEAD OF' ELSE 'AFTER' END, \
                OBJECT_DEFINITION(tr.object_id), tr.is_disabled \
         FROM sys.triggers tr \
         JOIN sys.objects o ON o.object_id = tr.parent_id \
         JOIN sys.schemas s ON s.schema_id = o.schema_id \
         WHERE s.name = @P1 AND o.name = @P2 \
         ORDER BY tr.name",
        &[
            fluxdb_core::SqlParameter::Text(schema.to_string()),
            fluxdb_core::SqlParameter::Text(table.to_string()),
        ],
    ))?;
    let rows = sqlserver_first_rows(&batch, "SQL Server 触发器元数据返回类型异常")?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(TriggerInfo {
                name: row.values.first().and_then(cell_text)?,
                event: row.values.get(1).and_then(cell_text).unwrap_or_default(),
                timing: row.values.get(2).and_then(cell_text).unwrap_or_default(),
                body: row.values.get(3).and_then(cell_text),
            })
        })
        .collect())
}

fn sqlserver_table_ddl_for(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<String> {
    let schema = sqlserver_request_schema(config, path)?;
    let database = sqlserver_request_database(config, path.database.as_deref())?;
    let mut client = sqlserver_connect_database(config, Some(&database))?;
    let ddl = sqlserver_table_ddl_with_client(&mut client, path, &schema);
    sqlserver_block_on(sqlserver_close(client));
    ddl
}

fn sqlserver_table_ddl_with_client(
    client: &mut SqlServerClient,
    path: &ObjectPath,
    schema: &str,
) -> fluxdb_core::Result<String> {
    if matches!(
        path.kind,
        ObjectKind::View | ObjectKind::Procedure | ObjectKind::Function
    ) {
        let batch = sqlserver_block_on(sqlserver_run_query(
            client,
            "SELECT OBJECT_DEFINITION(OBJECT_ID(@P1))",
            &[fluxdb_core::SqlParameter::Text(format!(
                "{}.{}",
                sqlserver_quote_identifier_for_object_id(schema),
                sqlserver_quote_identifier_for_object_id(&path.name)
            ))],
        ))?;
        let definition = sqlserver_first_rows(&batch, "SQL Server 视图 DDL 返回类型异常")?
            .first()
            .and_then(|row| row.values.first())
            .and_then(cell_text)
            .ok_or_else(|| Error::new(ErrorKind::Query, "未读取到 SQL Server 对象定义"))?;
        return Ok(format!("{};", definition.trim().trim_end_matches(';')));
    }

    let columns = sqlserver_load_columns(client, schema, &path.name)?;
    let indexes = sqlserver_load_indexes(client, schema, &path.name)?;
    let foreign_keys = sqlserver_load_foreign_keys(client, schema, &path.name)?;
    Ok(sqlserver_render_table_ddl(
        schema,
        &path.name,
        &columns,
        &indexes,
        &foreign_keys,
    ))
}

fn sqlserver_quote_identifier_for_object_id(value: &str) -> String {
    fluxdb_core::SqlServerDialect.quote_identifier(value)
}

fn sqlserver_render_table_ddl(
    schema: &str,
    table: &str,
    columns: &[SqlServerColumnInfo],
    indexes: &[IndexInfo],
    foreign_keys: &[ForeignKeyInfo],
) -> String {
    let table_name = sqlserver_qualified_table(schema, table);
    let mut definitions = columns
        .iter()
        .map(|column| {
            let mut definition = format!(
                "    {} {}",
                fluxdb_core::SqlServerDialect.quote_identifier(&column.name),
                column.data_type
            );
            if column.identity {
                definition.push_str(" IDENTITY(1,1)");
            }
            if !column.nullable {
                definition.push_str(" NOT NULL");
            }
            if let Some(default) = column.default_expr.as_deref().filter(|value| !value.trim().is_empty()) {
                definition.push_str(&format!(" DEFAULT {default}"));
            }
            definition
        })
        .collect::<Vec<_>>();
    let primary_key = columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| fluxdb_core::SqlServerDialect.quote_identifier(&column.name))
        .collect::<Vec<_>>();
    if !primary_key.is_empty() {
        definitions.push(format!("    PRIMARY KEY ({})", primary_key.join(", ")));
    }
    let mut ddl = format!("CREATE TABLE {table_name} (\n{}\n);", definitions.join(",\n"));
    for index in indexes.iter().filter(|index| !index.is_primary && !index.columns.is_empty()) {
        ddl.push_str(&format!(
            "\nCREATE {}INDEX {} ON {table_name} ({});",
            if index.is_unique { "UNIQUE " } else { "" },
            fluxdb_core::SqlServerDialect.quote_identifier(&index.name),
            index
                .columns
                .iter()
                .map(|column| fluxdb_core::SqlServerDialect.quote_identifier(column))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for fk in foreign_keys {
        ddl.push_str(&format!(
            "\nALTER TABLE {table_name} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {}.{} ({});",
            fluxdb_core::SqlServerDialect.quote_identifier(&fk.name),
            fk.columns
                .iter()
                .map(|column| fluxdb_core::SqlServerDialect.quote_identifier(column))
                .collect::<Vec<_>>()
                .join(", "),
            fluxdb_core::SqlServerDialect.quote_identifier(fk.ref_schema.as_deref().unwrap_or("dbo")),
            fluxdb_core::SqlServerDialect.quote_identifier(&fk.ref_table),
            fk.ref_columns
                .iter()
                .map(|column| fluxdb_core::SqlServerDialect.quote_identifier(column))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    ddl
}

fn sqlserver_load_table_design_metadata(
    config: &ConnectionConfig,
    path: &ObjectPath,
    include_details: bool,
) -> fluxdb_core::Result<fluxdb_core::TableDesignMetadata> {
    let schema = sqlserver_request_schema(config, path)?;
    let database = sqlserver_request_database(config, path.database.as_deref())?;
    let mut client = sqlserver_connect_database(config, Some(&database))?;
    let columns = sqlserver_load_columns(&mut client, &schema, &path.name)?;
    let indexes = if include_details {
        sqlserver_load_indexes(&mut client, &schema, &path.name)?
    } else {
        Vec::new()
    };
    let foreign_keys = if include_details {
        sqlserver_load_foreign_keys(&mut client, &schema, &path.name)?
    } else {
        Vec::new()
    };
    let triggers = if include_details {
        sqlserver_load_triggers(&mut client, &schema, &path.name)?
    } else {
        Vec::new()
    };
    let ddl = if path.kind == ObjectKind::View {
        sqlserver_table_ddl_with_client(&mut client, path, &schema)?
    } else {
        sqlserver_render_table_ddl(&schema, &path.name, &columns, &indexes, &foreign_keys)
    };
    let table_comment = sqlserver_load_table_comment(&mut client, &schema, &path.name)?;
    let column_defaults = columns
        .iter()
        .filter_map(|column| {
            column
                .default_expr
                .as_ref()
                .map(|expr| (column.name.clone(), expr.clone()))
        })
        .collect();
    let auto_increment_columns = columns
        .iter()
        .filter(|column| column.identity)
        .map(|column| column.name.clone())
        .collect();
    let (foreign_key_actions, checks) = if include_details {
        (
            sqlserver_load_foreign_key_actions(&mut client, &schema, &path.name)?,
            sqlserver_load_checks(&mut client, &schema, &path.name)?,
        )
    } else {
        (BTreeMap::new(), Vec::new())
    };
    let index_attributes = indexes
        .iter()
        .map(|index| {
            (
                index.name.clone(),
                fluxdb_core::IndexAttributes {
                    index_type: index.index_type.clone(),
                    predicate: None,
                    has_expression: false,
                },
            )
        })
        .collect();
    sqlserver_block_on(sqlserver_close(client));
    Ok(fluxdb_core::TableDesignMetadata {
        columns: sqlserver_completion_columns(path, columns),
        indexes,
        foreign_keys,
        triggers,
        column_defaults,
        auto_increment_columns,
        // 计算列表达式在本加载器里没有读取，留空即「未读取」。
        generated_columns: BTreeMap::new(),
        index_attributes,
        foreign_key_actions,
        checks,
        checks_loaded: include_details,
        table_comment,
        ddl,
        details_loaded: include_details,
    })
}

fn sqlserver_load_foreign_key_actions(
    client: &mut SqlServerClient,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<BTreeMap<String, fluxdb_core::ForeignKeyActions>> {
    let batch = sqlserver_block_on(sqlserver_run_query(
        client,
        "SELECT fk.name, fk.delete_referential_action_desc, fk.update_referential_action_desc \
         FROM sys.foreign_keys fk \
         JOIN sys.tables t ON t.object_id = fk.parent_object_id \
         JOIN sys.schemas s ON s.schema_id = t.schema_id \
         WHERE s.name = @P1 AND t.name = @P2",
        &[
            fluxdb_core::SqlParameter::Text(schema.to_string()),
            fluxdb_core::SqlParameter::Text(table.to_string()),
        ],
    ))?;
    let rows = sqlserver_first_rows(&batch, "SQL Server 外键动作返回类型异常")?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let name = row.values.first().and_then(cell_text)?;
            // `NO_ACTION` 之类的下划线写法统一成 SQL 关键字 `NO ACTION`。
            let action = |value: Option<String>| {
                value
                    .map(|text| text.replace('_', " ").to_ascii_uppercase())
                    .filter(|text| !text.is_empty())
            };
            Some((
                name,
                fluxdb_core::ForeignKeyActions {
                    on_delete: action(row.values.get(1).and_then(cell_text)),
                    on_update: action(row.values.get(2).and_then(cell_text)),
                },
            ))
        })
        .collect())
}

fn sqlserver_load_checks(
    client: &mut SqlServerClient,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<fluxdb_core::CheckMeta>> {
    let batch = sqlserver_block_on(sqlserver_run_query(
        client,
        "SELECT cc.name, cc.definition \
         FROM sys.check_constraints cc \
         JOIN sys.tables t ON t.object_id = cc.parent_object_id \
         JOIN sys.schemas s ON s.schema_id = t.schema_id \
         WHERE s.name = @P1 AND t.name = @P2 \
         ORDER BY cc.name",
        &[
            fluxdb_core::SqlParameter::Text(schema.to_string()),
            fluxdb_core::SqlParameter::Text(table.to_string()),
        ],
    ))?;
    let rows = sqlserver_first_rows(&batch, "SQL Server CHECK 约束返回类型异常")?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(fluxdb_core::CheckMeta {
                name: row.values.first().and_then(cell_text)?,
                expression: row.values.get(1).and_then(cell_text)?,
                definition: String::new(),
            })
        })
        .collect())
}

fn sqlserver_load_table_comment(
    client: &mut SqlServerClient,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Option<String>> {
    let batch = sqlserver_block_on(sqlserver_run_query(
        client,
        "SELECT CAST(ep.value AS nvarchar(max)) \
         FROM sys.objects o \
         JOIN sys.schemas s ON s.schema_id = o.schema_id \
         LEFT JOIN sys.extended_properties ep \
           ON ep.class = 1 AND ep.major_id = o.object_id AND ep.minor_id = 0 \
          AND ep.name = 'MS_Description' \
         WHERE s.name = @P1 AND o.name = @P2 AND o.type = 'U'",
        &[
            fluxdb_core::SqlParameter::Text(schema.to_string()),
            fluxdb_core::SqlParameter::Text(table.to_string()),
        ],
    ))?;
    let rows = sqlserver_first_rows(&batch, "SQL Server 表注释返回类型异常")?;
    Ok(rows
        .into_iter()
        .next()
        .and_then(|row| row.values.first().and_then(cell_text))
        .filter(|comment| !comment.trim().is_empty()))
}

fn sqlserver_completion_tables(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionTable>> {
    let database = sqlserver_request_database(config, database)?;
    // 无显式 schema 时扫描整个数据库：SQL Server 没有 search_path，
    // 只取登录默认 schema（通常是 dbo）会漏掉 app/hr/finance 等 schema 的表。
    let schema = schema
        .filter(|schema| !schema.trim().is_empty())
        .map(str::to_string);
    let mut client = sqlserver_connect_database(config, Some(&database))?;
    let filter_pattern = format!("%{}%", filter.trim());
    // `@Pn` 必须从 1 连续编号：按是否限定 schema 分别拼 SQL，避免占位符错位。
    let (sql, parameters) = match schema.as_deref() {
        Some(schema) => (
            "SELECT TOP (@P3) o.name, o.type, s.name, CAST(ep.value AS nvarchar(max)) AS comment \
             FROM sys.objects o \
             JOIN sys.schemas s ON s.schema_id = o.schema_id \
             LEFT JOIN sys.extended_properties ep \
               ON ep.class = 1 AND ep.major_id = o.object_id AND ep.minor_id = 0 AND ep.name = 'MS_Description' \
             WHERE s.name = @P1 AND o.type IN ('U','V') AND o.is_ms_shipped = 0 AND o.name LIKE @P2 \
             ORDER BY o.name"
                .to_string(),
            vec![
                fluxdb_core::SqlParameter::Text(schema.to_string()),
                fluxdb_core::SqlParameter::Text(filter_pattern),
                fluxdb_core::SqlParameter::I64(limit.to_string()),
            ],
        ),
        None => (
            "SELECT TOP (@P2) o.name, o.type, s.name, CAST(ep.value AS nvarchar(max)) AS comment \
             FROM sys.objects o \
             JOIN sys.schemas s ON s.schema_id = o.schema_id \
             LEFT JOIN sys.extended_properties ep \
               ON ep.class = 1 AND ep.major_id = o.object_id AND ep.minor_id = 0 AND ep.name = 'MS_Description' \
             WHERE s.name NOT IN ('guest','INFORMATION_SCHEMA','sys') \
               AND o.type IN ('U','V') AND o.is_ms_shipped = 0 AND o.name LIKE @P1 \
             ORDER BY s.name, o.name"
                .to_string(),
            vec![
                fluxdb_core::SqlParameter::Text(filter_pattern),
                fluxdb_core::SqlParameter::I64(limit.to_string()),
            ],
        ),
    };
    let batch = sqlserver_block_on(async {
        let batch = sqlserver_run_query(&mut client, &sql, &parameters).await?;
        sqlserver_close(client).await;
        Ok(batch)
    })?;
    let rows = sqlserver_first_rows(&batch, "SQL Server 补全表列表返回类型异常")?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let name = row.values.first().and_then(cell_text)?;
            let object_type = row.values.get(1).and_then(cell_text).unwrap_or_default();
            let table_schema = row.values.get(2).and_then(cell_text);
            Some(CompletionTable {
                database: Some(database.clone()),
                schema: table_schema.or_else(|| schema.clone()),
                name,
                kind: if object_type.trim().eq_ignore_ascii_case("V") {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                },
                comment: row.values.get(3).and_then(cell_text),
            })
        })
        .collect())
}

fn sqlserver_completion_columns_for_table(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    table: &str,
) -> fluxdb_core::Result<Vec<CompletionColumn>> {
    let database = sqlserver_request_database(config, database)?;
    let schema = schema
        .filter(|schema| !schema.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            config
                .sqlserver_profile
                .as_ref()
                .map(|profile| profile.basic.schema.clone())
                .filter(|schema| !schema.trim().is_empty())
        })
        .ok_or_else(|| Error::new(ErrorKind::Query, "缺少 SQL Server schema 上下文"))?;
    let mut client = sqlserver_connect_database(config, Some(&database))?;
    let columns = sqlserver_load_columns(&mut client, &schema, table);
    sqlserver_block_on(sqlserver_close(client));
    let path = ObjectPath {
        connection_id: config.id,
        database: Some(database),
        schema: Some(schema),
        name: table.to_string(),
        kind: ObjectKind::Table,
    };
    Ok(sqlserver_completion_columns(&path, columns?))
}

/// SQL Server 可补全的例程：存储过程与标量/表值函数。
///
/// `sys.objects` 的 `type` 编码：P/PC 存储过程；FN/IF/TF/FS/FT 函数。
/// 同名重载在 SQL Server 里不合法（同一 schema 内名称唯一），签名只作展示，
/// 不作为去重身份。
fn sqlserver_completion_routines(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    filter: &str,
    limit: u64,
) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
    let database = sqlserver_request_database(config, database)?;
    let schema = sqlserver_completion_schema(config, schema)?;
    let mut client = sqlserver_connect_database(config, Some(&database))?;
    let batch = sqlserver_block_on(async {
        let batch = sqlserver_run_query(
            &mut client,
            "SELECT TOP (@P3) o.name, o.type, \
                    COALESCE(STUFF((SELECT N', ' + p.name + N' ' + TYPE_NAME(p.user_type_id) \
                                     FROM sys.parameters p \
                                     WHERE p.object_id = o.object_id AND p.parameter_id > 0 \
                                     ORDER BY p.parameter_id \
                                     FOR XML PATH(''), TYPE).value('.', 'nvarchar(max)'), 1, 2, N''), N'') AS signature \
             FROM sys.objects o \
             JOIN sys.schemas s ON s.schema_id = o.schema_id \
             WHERE s.name = @P1 \
               AND o.type IN ('P','PC','FN','IF','TF','FS','FT') \
               AND o.is_ms_shipped = 0 \
               AND o.name LIKE @P2",
            &[
                fluxdb_core::SqlParameter::Text(schema.clone()),
                fluxdb_core::SqlParameter::Text(format!("%{}%", filter.trim())),
                fluxdb_core::SqlParameter::I64(limit.to_string()),
            ],
        )
        .await?;
        sqlserver_close(client).await;
        Ok(batch)
    })?;
    Ok(sqlserver_first_rows(&batch, "SQL Server 补全例程返回类型异常")?
        .into_iter()
        .filter_map(|row| {
            let name = row.values.first().and_then(cell_text)?;
            let object_type = row.values.get(1).and_then(cell_text).unwrap_or_default();
            let signature = row
                .values
                .get(2)
                .and_then(cell_text)
                .filter(|signature| !signature.trim().is_empty());
            Some(CompletionRoutine {
                schema: Some(schema.clone()),
                name,
                kind: if matches!(object_type.trim(), "P" | "PC") {
                    CompletionRoutineKind::Procedure
                } else {
                    CompletionRoutineKind::Function
                },
                signature,
            })
        })
        .collect())
}

fn sqlserver_completion_schema(
    config: &ConnectionConfig,
    schema: Option<&str>,
) -> fluxdb_core::Result<String> {
    schema
        .filter(|schema| !schema.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            config
                .sqlserver_profile
                .as_ref()
                .map(|profile| profile.basic.schema.clone())
                .filter(|schema| !schema.trim().is_empty())
        })
        .ok_or_else(|| Error::new(ErrorKind::Query, "缺少 SQL Server schema 上下文"))
}
