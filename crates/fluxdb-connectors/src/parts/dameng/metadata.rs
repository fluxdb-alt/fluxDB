/// 达梦会为外键等约束创建 `SREF_CON_*` 内部表，另有 `##` 前缀的临时/系统对象。
/// 它们不对应可浏览的用户关系，补全与对象树统一隐藏，避免生成不可引用的候选。
fn dm_internal_object_name(name: &str) -> bool {
    let name = name.trim().to_ascii_uppercase();
    name.starts_with("SREF_CON_") || name.starts_with("##")
}

#[cfg(test)]
mod dameng_internal_object_tests {
    use super::dm_internal_object_name;

    #[test]
    fn hides_dameng_foreign_key_and_temporary_tables() {
        assert!(dm_internal_object_name("SREF_CON_TAB134218794_LEVEL"));
        assert!(dm_internal_object_name("##DM_TEMP_TABLE"));
        assert!(!dm_internal_object_name("CUSTOMERS"));
        assert!(!dm_internal_object_name("V_CUSTOMER_VALUE"));
    }
}

fn dm_list_objects(
    config: &ConnectionConfig,
    path: Option<&ObjectPath>,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    if config.kind != DatabaseKind::Dameng {
        return Err(Error::new(ErrorKind::Connection, "连接类型不匹配"));
    }
    match path {
        None => dm_list_databases(config),
        Some(path) if path.kind == ObjectKind::Database => dm_list_schemas(config, path),
        Some(path) if path.kind == ObjectKind::Schema => dm_list_relations(config, path),
        Some(_) => Err(Error::new(ErrorKind::Internal, "不支持的达梦对象层级")),
    }
}

fn dm_list_databases(config: &ConnectionConfig) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let mut client = dm_connect(config)?;
    let result = dm_execute(&mut client, "SELECT NAME FROM V$DATABASE", &[])?;
    client.close();
    let ExecuteResult::Query { rows, .. } = result else {
        return Err(Error::new(ErrorKind::Query, "达梦数据库列表返回类型异常"));
    };
    rows.into_iter()
        .filter_map(|row| row.first().and_then(dm_value_text))
        .map(|name| {
            Ok(ObjectSummary {
                path: ObjectPath {
                    connection_id: config.id,
                    database: Some(name.clone()),
                    schema: None,
                    name,
                    kind: ObjectKind::Database,
                },
                rows: None,
                size_bytes: None,
                modified_at: None,
                comment: None,
                stable: None,
            })
        })
        .collect()
}

fn dm_list_schemas(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let database = path
        .database
        .clone()
        .or_else(|| path.name.clone().into());
    let mut client = dm_connect(config)?;
    let result = dm_execute(
        &mut client,
        "SELECT NAME FROM SYS.SYSOBJECTS WHERE TYPE$='SCH' ORDER BY NAME",
        &[],
    )?;
    client.close();
    let ExecuteResult::Query { rows, .. } = result else {
        return Err(Error::new(ErrorKind::Query, "达梦 schema 列表返回类型异常"));
    };
    rows.into_iter()
        .filter_map(|row| row.first().and_then(dm_value_text))
        .map(|name| {
            Ok(ObjectSummary {
                path: ObjectPath {
                    connection_id: config.id,
                    database: database.clone(),
                    schema: None,
                    name,
                    kind: ObjectKind::Schema,
                },
                rows: None,
                size_bytes: None,
                modified_at: None,
                comment: None,
                stable: None,
            })
        })
        .collect()
}

fn dm_list_relations(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let database = path.database.clone();
    let schema = path.name.clone();
    let mut client = dm_connect(config)?;
    let result = dm_execute(
        &mut client,
        "SELECT T.NAME, T.SUBTYPE$, T.ID, C.COMMENT$ \
         FROM SYS.SYSOBJECTS T \
         JOIN SYS.SYSOBJECTS S ON S.ID=T.SCHID \
         LEFT JOIN SYS.SYSTABLECOMMENTS C \
           ON C.SCHNAME=S.NAME AND C.TVNAME=T.NAME \
         WHERE S.NAME=? AND T.TYPE$='SCHOBJ' AND T.SUBTYPE$ IN ('UTAB','VIEW') \
         ORDER BY T.NAME",
        &[Param::Str(schema.clone())],
    )?;
    client.close();
    let ExecuteResult::Query { rows, .. } = result else {
        return Err(Error::new(ErrorKind::Query, "达梦对象列表返回类型异常"));
    };
    let mut objects = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(name) = row.first().and_then(dm_value_text) else {
            continue;
        };
        if dm_internal_object_name(&name) {
            continue;
        }
        let object_type = row.get(1).and_then(dm_value_text).unwrap_or_default();
        let stable = row.get(2).and_then(dm_value_u64);
        let comment = row.get(3).and_then(dm_value_text);
        objects.push(ObjectSummary {
            path: ObjectPath {
                connection_id: config.id,
                database: database.clone(),
                schema: Some(schema.clone()),
                name,
                kind: if object_type.eq_ignore_ascii_case("VIEW") {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                },
            },
            rows: None,
            size_bytes: None,
            modified_at: None,
            comment,
            stable,
        });
    }
    Ok(objects)
}

#[derive(Clone, Debug)]
struct DmColumnInfo {
    name: String,
    type_name: String,
    nullable: bool,
    primary_key: bool,
    comment: Option<String>,
    default_value: Option<String>,
    auto_increment: bool,
    primary_key_constraint: Option<String>,
}

fn dm_load_columns(
    client: &mut DmClient,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<Column>> {
    Ok(dm_load_column_infos(client, schema, table)?
        .into_iter()
        .map(|column| Column {
            name: column.name,
            type_name: Some(column.type_name),
            nullable: column.nullable,
            primary_key: column.primary_key,
            comment: column.comment,
        })
        .collect())
}

fn dm_load_column_infos(
    client: &mut DmClient,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<DmColumnInfo>> {
    // `ALL_TAB_COLUMNS` 在 DM8 上会展开全部所有者，单表打开也需数百毫秒。
    // 字段与注释按对象 ID 走 SYS 底表；主键/身份/约束继续用稳定视图补齐。
    let fast_result = dm_execute(
        client,
        "SELECT C.NAME, C.TYPE$, C.LENGTH$, C.SCALE, C.NULLABLE$, CC.COMMENT$, C.DEFVAL, C.INFO2 \
         FROM SYS.SYSCOLUMNS C \
         JOIN SYS.SYSOBJECTS T ON T.ID=C.ID \
         JOIN SYS.SYSOBJECTS S ON S.ID=T.SCHID \
         LEFT JOIN SYS.SYSCOLUMNCOMMENTS CC \
           ON CC.SCHNAME=S.NAME AND CC.TVNAME=T.NAME AND CC.COLNAME=C.NAME \
         WHERE S.NAME=? AND T.NAME=? AND T.SUBTYPE$='UTAB' \
         ORDER BY C.COLID",
        &[Param::Str(schema.to_string()), Param::Str(table.to_string())],
    )?;
    let ExecuteResult::Query { rows, .. } = fast_result else {
        return Err(Error::new(ErrorKind::Query, "达梦列元数据返回类型异常"));
    };
    if rows.is_empty() {
        return dm_load_column_infos_from_catalog_views(client, schema, table);
    }
    let constraints = dm_execute(
        client,
        "SELECT CO.CONSTRAINT_NAME, CC.COLUMN_NAME, CO.CONSTRAINT_TYPE \
         FROM ALL_CONSTRAINTS CO JOIN ALL_CONS_COLUMNS CC \
           ON CC.OWNER=CO.OWNER AND CC.CONSTRAINT_NAME=CO.CONSTRAINT_NAME \
         WHERE CO.OWNER=? AND CO.TABLE_NAME=? AND CO.CONSTRAINT_TYPE='P'",
        &[Param::Str(schema.to_string()), Param::Str(table.to_string())],
    )?;
    let ExecuteResult::Query {
        rows: constraint_rows,
        ..
    } = constraints
    else {
        return Err(Error::new(ErrorKind::Query, "达梦主键元数据返回类型异常"));
    };
    let primary_keys: std::collections::BTreeMap<String, String> = constraint_rows
        .into_iter()
        .filter_map(|row| {
            Some((
                row.get(1).and_then(dm_value_text)?.to_ascii_uppercase(),
                row.first().and_then(dm_value_text)?,
            ))
        })
        .collect();
    rows.into_iter()
        .map(|row| {
            let name = row.first().and_then(dm_value_text).ok_or_else(|| {
                Error::new(ErrorKind::Query, "达梦列元数据缺少字段名")
            })?;
            let data_type = row.get(1).and_then(dm_value_text).unwrap_or_default();
            let length = row.get(2).and_then(dm_meta_u64).unwrap_or(0);
            let scale = row.get(3).and_then(dm_meta_u64).unwrap_or(0);
            let primary_key_constraint =
                primary_keys.get(&name.to_ascii_uppercase()).cloned();
            Ok(DmColumnInfo {
                name: name.clone(),
                type_name: dm_column_type_name(
                    &data_type,
                    length,
                    dm_numeric_precision_from_length(&data_type, length),
                    scale,
                ),
                nullable: row
                    .get(4)
                    .and_then(dm_value_text)
                    .is_some_and(|value| value.eq_ignore_ascii_case("Y")),
                primary_key: primary_key_constraint.is_some(),
                comment: row.get(5).and_then(dm_value_text),
                default_value: row
                    .get(6)
                    .and_then(dm_value_text)
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL")),
                auto_increment: row.get(7).and_then(dm_meta_i64).unwrap_or(0) != 0,
                primary_key_constraint,
            })
        })
        .collect()
}

fn dm_load_column_infos_from_catalog_views(
    client: &mut DmClient,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<DmColumnInfo>> {
    let result = dm_execute(
        client,
        "SELECT C.COLUMN_NAME, C.DATA_TYPE, C.DATA_LENGTH, C.DATA_PRECISION, C.DATA_SCALE, \
                C.NULLABLE, CC.COMMENTS, \
                CASE WHEN PK.COLUMN_NAME IS NULL THEN 0 ELSE 1 END AS IS_PK, \
                C.DATA_DEFAULT, \
                CASE WHEN EXISTS ( \
                    SELECT 1 FROM SYS.SYSCOLUMNS SC \
                    JOIN SYS.SYSOBJECTS T ON SC.ID=T.ID \
                    JOIN SYS.SYSOBJECTS S ON T.SCHID=S.ID \
                    WHERE S.NAME=C.OWNER AND T.NAME=C.TABLE_NAME \
                      AND SC.NAME=C.COLUMN_NAME AND (SC.INFO2 & 0x01)=0x01 \
                ) THEN 1 ELSE 0 END AS IS_IDENTITY, \
                PK.CONSTRAINT_NAME \
         FROM ALL_TAB_COLUMNS C \
         LEFT JOIN ALL_COL_COMMENTS CC ON CC.OWNER=C.OWNER AND CC.TABLE_NAME=C.TABLE_NAME \
                                      AND CC.COLUMN_NAME=C.COLUMN_NAME \
         LEFT JOIN (SELECT CO.OWNER, CO.TABLE_NAME, CC.COLUMN_NAME, CO.CONSTRAINT_NAME \
                    FROM ALL_CONSTRAINTS CO JOIN ALL_CONS_COLUMNS CC \
                      ON CC.OWNER=CO.OWNER AND CC.CONSTRAINT_NAME=CO.CONSTRAINT_NAME \
                    WHERE CO.CONSTRAINT_TYPE='P') PK \
           ON PK.OWNER=C.OWNER AND PK.TABLE_NAME=C.TABLE_NAME AND PK.COLUMN_NAME=C.COLUMN_NAME \
         WHERE C.OWNER=? AND C.TABLE_NAME=? ORDER BY C.COLUMN_ID",
        &[Param::Str(schema.to_string()), Param::Str(table.to_string())],
    )?;
    let ExecuteResult::Query { rows, .. } = result else {
        return Err(Error::new(ErrorKind::Query, "达梦列元数据返回类型异常"));
    };
    rows.into_iter()
        .map(|row| {
            let name = row.first().and_then(dm_value_text).ok_or_else(|| {
                Error::new(ErrorKind::Query, "达梦列元数据缺少字段名")
            })?;
            let data_type = row.get(1).and_then(dm_value_text).unwrap_or_default();
            let length = row.get(2).and_then(dm_meta_u64).unwrap_or(0);
            let precision = row.get(3).and_then(dm_meta_u64);
            let scale = row.get(4).and_then(dm_meta_u64).unwrap_or(0);
            let type_name = dm_column_type_name(&data_type, length, precision, scale);
            Ok(DmColumnInfo {
                name,
                type_name,
                nullable: row
                    .get(5)
                    .and_then(dm_value_text)
                    .is_some_and(|value| value.eq_ignore_ascii_case("Y")),
                primary_key: row.get(7).and_then(dm_meta_i64).unwrap_or(0) != 0,
                comment: row.get(6).and_then(dm_value_text),
                default_value: row
                    .get(8)
                    .and_then(dm_value_text)
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL")),
                auto_increment: row.get(9).and_then(dm_meta_i64).unwrap_or(0) != 0,
                primary_key_constraint: row.get(10).and_then(dm_value_text),
            })
        })
        .collect()
}

fn dm_column_type_name(
    data_type: &str,
    length: u64,
    precision: Option<u64>,
    scale: u64,
) -> String {
    let upper = data_type.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "VARCHAR" | "VARCHAR2" | "CHAR" | "NVARCHAR" | "NVARCHAR2" | "NCHAR"
            | "BINARY" | "VARBINARY"
    ) {
        if length == 0 {
            return data_type.to_string();
        }
        format!("{data_type}({length})")
    } else if matches!(upper.as_str(), "DECIMAL" | "NUMERIC" | "NUMBER") {
        precision
            .map(|precision| format!("{data_type}({precision},{scale})"))
            .unwrap_or_else(|| data_type.to_string())
    } else {
        data_type.to_string()
    }
}

/// SYSCOLUMNS 对 DECIMAL/NUMERIC 不单独保存 precision；LENGTH$ 即总精度位数。
fn dm_numeric_precision_from_length(data_type: &str, length: u64) -> Option<u64> {
    matches!(
        data_type.to_ascii_uppercase().as_str(),
        "DECIMAL" | "NUMERIC" | "NUMBER"
    )
    .then_some(length)
    .filter(|length| *length > 0)
}

fn dm_indexes_for(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<IndexInfo>> {
    let schema = dm_request_schema(config, path)?;
    let mut client = dm_connect(config)?;
    let indexes = dm_load_indexes(&mut client, &schema, &path.name)?;
    client.close();
    Ok(indexes)
}

fn dm_load_indexes(
    client: &mut DmClient,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<IndexInfo>> {
    let result = dm_execute(
        client,
        "SELECT I.INDEX_NAME, I.UNIQUENESS, I.INDEX_TYPE, IC.COLUMN_NAME, IC.COLUMN_POSITION, \
                CASE WHEN C.CONSTRAINT_TYPE='P' THEN 1 ELSE 0 END AS IS_PK \
         FROM ALL_INDEXES I \
         LEFT JOIN ALL_IND_COLUMNS IC ON IC.INDEX_OWNER=I.OWNER AND IC.INDEX_NAME=I.INDEX_NAME \
         LEFT JOIN ALL_CONSTRAINTS C ON C.OWNER=I.OWNER AND C.INDEX_NAME=I.INDEX_NAME \
         WHERE I.OWNER=? AND I.TABLE_NAME=? ORDER BY I.INDEX_NAME, IC.COLUMN_POSITION",
        &[Param::Str(schema.to_string()), Param::Str(table.to_string())],
    )?;
    let ExecuteResult::Query { rows, .. } = result else {
        return Err(Error::new(ErrorKind::Query, "达梦索引元数据返回类型异常"));
    };
    let mut indexes: Vec<IndexInfo> = Vec::new();
    for row in rows {
        let name = row.first().and_then(dm_value_text).unwrap_or_default();
        let unique = row
            .get(1)
            .and_then(dm_value_text)
            .is_some_and(|value| value.eq_ignore_ascii_case("UNIQUE"));
        let index_type = row.get(2).and_then(dm_value_text);
        let column = row.get(3).and_then(dm_value_text);
        let is_primary = row.get(5).and_then(dm_meta_i64).unwrap_or(0) != 0;
        // 达梦的表组织索引（CLUSTER/HEAP 等）在 ALL_IND_COLUMNS 里没有列记录，
        // 直接吐出去会让 DDL 生成出 `CREATE INDEX ... ()` 这种非法语句。
        let has_columns = column.is_some();
        if let Some(index) = indexes.iter_mut().find(|index| index.name == name) {
            if let Some(column) = column
                && has_columns
                && !index
                    .columns
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(&column))
            {
                index.columns.push(column);
            }
            continue;
        }
        if !has_columns && !is_primary {
            continue;
        }
        indexes.push(IndexInfo {
            name,
            columns: column.into_iter().collect(),
            is_unique: unique,
            is_primary,
            index_type,
            comment: None,
        });
    }
    Ok(indexes)
}

fn dm_foreign_keys_for(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
    let schema = dm_request_schema(config, path)?;
    let mut client = dm_connect(config)?;
    let foreign_keys = dm_load_foreign_keys(&mut client, &schema, &path.name)?;
    client.close();
    Ok(foreign_keys)
}

fn dm_load_foreign_keys(
    client: &mut DmClient,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
    let result = dm_execute(
        client,
        "SELECT C.CONSTRAINT_NAME, CC.COLUMN_NAME, R.OWNER, R.TABLE_NAME, RCC.COLUMN_NAME, CC.POSITION \
         FROM ALL_CONSTRAINTS C \
         JOIN ALL_CONS_COLUMNS CC ON CC.OWNER=C.OWNER AND CC.CONSTRAINT_NAME=C.CONSTRAINT_NAME \
         JOIN ALL_CONSTRAINTS R ON R.OWNER=C.R_OWNER AND R.CONSTRAINT_NAME=C.R_CONSTRAINT_NAME \
         JOIN ALL_CONS_COLUMNS RCC ON RCC.OWNER=R.OWNER AND RCC.CONSTRAINT_NAME=R.CONSTRAINT_NAME \
                                  AND RCC.POSITION=CC.POSITION \
         WHERE C.OWNER=? AND C.TABLE_NAME=? AND C.CONSTRAINT_TYPE='R' \
         ORDER BY C.CONSTRAINT_NAME, CC.POSITION",
        &[Param::Str(schema.to_string()), Param::Str(table.to_string())],
    )?;
    let ExecuteResult::Query { rows, .. } = result else {
        return Err(Error::new(ErrorKind::Query, "达梦外键元数据返回类型异常"));
    };
    let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
    for row in rows {
        let name = row.first().and_then(dm_value_text).unwrap_or_default();
        let column = row.get(1).and_then(dm_value_text).unwrap_or_default();
        let ref_schema = row.get(2).and_then(dm_value_text);
        let ref_table = row.get(3).and_then(dm_value_text).unwrap_or_default();
        let ref_column = row.get(4).and_then(dm_value_text).unwrap_or_default();
        if let Some(fk) = foreign_keys.iter_mut().find(|fk| fk.name == name) {
            fk.columns.push(column.clone());
            fk.ref_columns.push(ref_column);
            fk.column = fk.columns.first().cloned().unwrap_or(column);
            fk.ref_column = fk.ref_columns.first().cloned().unwrap_or_default();
        } else {
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
    }
    Ok(foreign_keys)
}

fn dm_triggers_for(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<TriggerInfo>> {
    let schema = dm_request_schema(config, path)?;
    let mut client = dm_connect(config)?;
    let triggers = dm_load_triggers(&mut client, &schema, &path.name)?;
    client.close();
    Ok(triggers)
}

fn dm_load_triggers(
    client: &mut DmClient,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<TriggerInfo>> {
    let result = dm_execute(
        client,
        "SELECT TRIGGER_NAME, TRIGGERING_EVENT, \
                CASE WHEN TRIGGERING_TYPE LIKE 'BEFORE%' THEN 'BEFORE' \
                     WHEN TRIGGERING_TYPE LIKE 'AFTER%' THEN 'AFTER' \
                     ELSE TRIGGERING_TYPE END, TRIGGER_BODY \
         FROM ALL_TRIGGERS WHERE TABLE_OWNER=? AND TABLE_NAME=? ORDER BY TRIGGER_NAME",
        &[Param::Str(schema.to_string()), Param::Str(table.to_string())],
    )?;
    let ExecuteResult::Query { rows, .. } = result else {
        return Err(Error::new(ErrorKind::Query, "达梦触发器元数据返回类型异常"));
    };
    rows.into_iter()
        .map(|row| {
            let name = row.first().and_then(dm_value_text).ok_or_else(|| {
                Error::new(ErrorKind::Query, "达梦触发器元数据缺少名称")
            })?;
            Ok(TriggerInfo {
                name,
                event: row.get(1).and_then(dm_value_text).unwrap_or_default(),
                timing: row.get(2).and_then(dm_value_text).unwrap_or_default(),
                body: row.get(3).and_then(dm_value_text),
            })
        })
        .collect()
}

fn dm_table_ddl_for(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<String> {
    let schema = dm_request_schema(config, path)?;
    if path.kind == ObjectKind::View {
        return dm_view_ddl_for(config, &schema, &path.name);
    }
    let mut client = dm_connect(config)?;
    let columns = dm_load_column_infos(&mut client, &schema, &path.name)?;
    let indexes = dm_load_indexes(&mut client, &schema, &path.name)?;
    let foreign_keys = dm_load_foreign_keys(&mut client, &schema, &path.name)?;
    let table_comment = dm_load_table_comment(&mut client, &schema, &path.name)?;
    client.close();
    Ok(dm_render_table_ddl(
        &schema,
        &path.name,
        &columns,
        &indexes,
        &foreign_keys,
        table_comment.as_deref(),
    ))
}

/// 设计表专用快照：一次 DM8 登录内读取全部元数据，避免 UI 打开时重复握手。
fn dm_load_table_design_metadata(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<fluxdb_core::TableDesignMetadata> {
    dm_load_table_design_metadata_with_details(config, path, true)
}

fn dm_load_table_design_base_metadata(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<fluxdb_core::TableDesignMetadata> {
    dm_load_table_design_metadata_with_details(config, path, false)
}

fn dm_load_table_design_metadata_with_details(
    config: &ConnectionConfig,
    path: &ObjectPath,
    include_details: bool,
) -> fluxdb_core::Result<fluxdb_core::TableDesignMetadata> {
    let schema = dm_request_schema(config, path)?;
    if path.kind == ObjectKind::View {
        let mut client = dm_connect(config)?;
        let ddl = dm_view_ddl_with_client(&mut client, &schema, &path.name)?;
        let columns = dm_load_column_infos(&mut client, &schema, &path.name)?;
        client.close();
        return Ok(fluxdb_core::TableDesignMetadata {
            columns: dm_completion_columns(path, &schema, columns),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
            triggers: Vec::new(),
            ddl,
            details_loaded: true,
        });
    }

    let mut client = dm_connect(config)?;
    let columns = dm_load_column_infos(&mut client, &schema, &path.name)?;
    let indexes = if include_details {
        dm_load_indexes(&mut client, &schema, &path.name)?
    } else {
        Vec::new()
    };
    let foreign_keys = if include_details {
        dm_load_foreign_keys(&mut client, &schema, &path.name)?
    } else {
        Vec::new()
    };
    let triggers = if include_details {
        dm_load_triggers(&mut client, &schema, &path.name)?
    } else {
        Vec::new()
    };
    let table_comment = dm_load_table_comment(&mut client, &schema, &path.name)?;
    client.close();
    let ddl = if include_details {
        dm_render_table_ddl(
            &schema,
            &path.name,
            &columns,
            &indexes,
            &foreign_keys,
            table_comment.as_deref(),
        )
    } else {
        dm_render_table_base_ddl(
            &schema,
            &path.name,
            &columns,
            table_comment.as_deref(),
        )
    };
    Ok(fluxdb_core::TableDesignMetadata {
        columns: dm_completion_columns(path, &schema, columns),
        indexes,
        foreign_keys,
        triggers,
        ddl,
        details_loaded: include_details,
    })
}

fn dm_render_table_base_ddl(
    schema: &str,
    table_name: &str,
    columns: &[DmColumnInfo],
    table_comment: Option<&str>,
) -> String {
    dm_render_table_ddl(schema, table_name, columns, &[], &[], table_comment)
}

fn dm_completion_columns(
    path: &ObjectPath,
    schema: &str,
    columns: Vec<DmColumnInfo>,
) -> Vec<fluxdb_core::CompletionColumn> {
    columns
        .into_iter()
        .map(|column| fluxdb_core::CompletionColumn {
            database: path.database.clone(),
            schema: Some(schema.to_string()),
            table: path.name.clone(),
            name: column.name,
            type_name: Some(column.type_name),
            nullable: column.nullable,
            primary_key: column.primary_key,
            comment: column.comment,
            stable: None,
        })
        .collect()
}

fn dm_load_table_comment(
    client: &mut DmClient,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Option<String>> {
    let comments = dm_execute(
        client,
        "SELECT COMMENTS FROM ALL_TAB_COMMENTS WHERE OWNER=? AND TABLE_NAME=?",
        &[Param::Str(schema.to_string()), Param::Str(table.to_string())],
    )?;
    Ok(match comments {
        ExecuteResult::Query { rows, .. } => rows
            .first()
            .and_then(|row| row.first())
            .and_then(dm_value_text),
        _ => None,
    })
}

fn dm_render_table_ddl(
    schema: &str,
    table_name: &str,
    columns: &[DmColumnInfo],
    indexes: &[IndexInfo],
    foreign_keys: &[ForeignKeyInfo],
    table_comment: Option<&str>,
) -> String {
    let table = dm_qualified_table(schema, table_name);
    let mut lines = Vec::new();
    for column in columns {
        let mut line = format!(
            "  {} {}",
            fluxdb_core::DamengDialect.quote_identifier(&column.name),
            column.type_name
        );
        if column.auto_increment {
            line.push_str(" IDENTITY(1,1)");
        }
        if !column.nullable {
            line.push_str(" NOT NULL");
        }
        if let Some(default_value) = column.default_value.as_deref() {
            line.push_str(" DEFAULT ");
            line.push_str(default_value);
        }
        lines.push(line);
    }
    let pk_columns = columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| fluxdb_core::DamengDialect.quote_identifier(&column.name))
        .collect::<Vec<_>>();
    if !pk_columns.is_empty() {
        let constraint = columns
            .iter()
            .find_map(|column| column.primary_key_constraint.as_deref())
            .filter(|name| !name.trim().is_empty())
            .map(|name| {
                format!(
                    "CONSTRAINT {} ",
                    fluxdb_core::DamengDialect.quote_identifier(name)
                )
            })
            .unwrap_or_default();
        lines.push(format!(
            "  {constraint}PRIMARY KEY ({})",
            pk_columns.join(", ")
        ));
    }
    for fk in foreign_keys {
        if fk.columns.len() != fk.ref_columns.len() || fk.columns.is_empty() {
            continue;
        }
        let source = fk
            .columns
            .iter()
            .map(|column| fluxdb_core::DamengDialect.quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let target = fk
            .ref_columns
            .iter()
            .map(|column| fluxdb_core::DamengDialect.quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let ref_table = dm_qualified_table(
            fk.ref_schema.as_deref().unwrap_or(schema),
            &fk.ref_table,
        );
        lines.push(format!(
            "  CONSTRAINT {} FOREIGN KEY ({source}) REFERENCES {ref_table} ({target})",
            fluxdb_core::DamengDialect.quote_identifier(&fk.name)
        ));
    }
    let mut ddl = format!("CREATE TABLE {table} (\n{}\n)", lines.join(",\n"));
    for index in indexes
        .iter()
        .filter(|index| !index.is_primary && !index.columns.is_empty())
    {
        let cols = index
            .columns
            .iter()
            .map(|column| fluxdb_core::DamengDialect.quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        ddl.push_str(&format!(
            ";\nCREATE {}INDEX {} ON {table} ({cols})",
            if index.is_unique { "UNIQUE " } else { "" },
            fluxdb_core::DamengDialect.quote_identifier(&index.name)
        ));
    }
    if let Some(comment) = table_comment.filter(|comment| !comment.trim().is_empty()) {
        ddl.push_str(&format!(
            ";\nCOMMENT ON TABLE {table} IS {}",
            dm_string_literal(comment)
        ));
    }
    for column in columns.iter().filter(|column| column.comment.is_some()) {
        ddl.push_str(&format!(
            ";\nCOMMENT ON COLUMN {table}.{} IS {}",
            fluxdb_core::DamengDialect.quote_identifier(&column.name),
            dm_string_literal(column.comment.as_deref().unwrap_or_default())
        ));
    }
    format!("{ddl};")
}

fn dm_view_ddl_for(
    config: &ConnectionConfig,
    schema: &str,
    view: &str,
) -> fluxdb_core::Result<String> {
    let mut client = dm_connect(config)?;
    let ddl = dm_view_ddl_with_client(&mut client, schema, view)?;
    client.close();
    Ok(ddl)
}

fn dm_view_ddl_with_client(
    client: &mut DmClient,
    schema: &str,
    view: &str,
) -> fluxdb_core::Result<String> {
    let result = dm_execute(
        client,
        "SELECT TEXT FROM ALL_VIEWS WHERE OWNER=? AND VIEW_NAME=?",
        &[Param::Str(schema.to_string()), Param::Str(view.to_string())],
    )?;
    let ExecuteResult::Query { rows, .. } = result else {
        return Err(Error::new(ErrorKind::Query, "达梦视图 DDL 返回类型异常"));
    };
    let text = rows
        .first()
        .and_then(|row| row.first())
        .and_then(dm_value_text)
        .ok_or_else(|| Error::new(ErrorKind::Query, "未读取到达梦视图定义"))?;
    Ok(format!(
        "CREATE OR REPLACE VIEW {} AS\n{};",
        dm_qualified_table(schema, view),
        text.trim().trim_end_matches(';').trim_end()
    ))
}

fn dm_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod dameng_fast_metadata_tests {
    use super::{dm_column_type_name, dm_numeric_precision_from_length};

    #[test]
    fn syscolumns_length_keeps_numeric_precision_and_scale() {
        for data_type in ["DECIMAL", "NUMERIC", "NUMBER"] {
            let precision = dm_numeric_precision_from_length(data_type, 14);
            assert_eq!(
                dm_column_type_name(data_type, 14, precision, 2),
                format!("{data_type}(14,2)")
            );
        }
        assert_eq!(dm_numeric_precision_from_length("INT", 4), None);
    }
}
