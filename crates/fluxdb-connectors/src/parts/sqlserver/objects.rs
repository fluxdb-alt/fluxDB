/// SQL Server 对象树按 database → schema → table/view 三层展开。
/// `Database` 节点本身仍携带连接 id；`Schema` 节点保留 database，表节点再补 schema。
fn sqlserver_list_objects(
    config: &ConnectionConfig,
    path: Option<&ObjectPath>,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    match path {
        None => sqlserver_list_databases(config),
        Some(path) if path.kind == ObjectKind::Database => sqlserver_list_schemas(config, path),
        Some(path) if path.kind == ObjectKind::Schema => sqlserver_list_relations(config, path),
        Some(_) => Err(Error::new(
            ErrorKind::Internal,
            "不支持的 SQL Server 对象层级",
        )),
    }
}

fn sqlserver_list_databases(config: &ConnectionConfig) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let mut client = sqlserver_connect(config)?;
    let batch = sqlserver_block_on(async {
        let batch = sqlserver_run_batch(
            &mut client,
            "SELECT name FROM sys.databases WHERE state = 0 AND HAS_DBACCESS(name) = 1 ORDER BY name",
        )
        .await?;
        sqlserver_close(client).await;
        Ok(batch)
    })?;
    let Some(rows) = batch.result_sets.into_iter().next().map(|set| set.rows) else {
        return Err(Error::new(ErrorKind::Query, "SQL Server 数据库列表返回类型异常"));
    };
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let name = row.values.first().and_then(cell_text)?;
            Some(ObjectSummary {
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
        .collect())
}

fn sqlserver_list_schemas(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let database = path
        .database
        .clone()
        .filter(|database| !database.trim().is_empty())
        .unwrap_or_else(|| path.name.clone());
    let mut client = sqlserver_connect_database(config, Some(&database))?;
    let batch = sqlserver_block_on(async {
        let batch = sqlserver_run_batch(
            &mut client,
            "SELECT s.name \
             FROM sys.schemas s \
             WHERE s.name NOT IN ('guest','INFORMATION_SCHEMA','sys') \
               AND s.name NOT LIKE 'db[_]%' \
             ORDER BY s.name",
        )
        .await?;
        sqlserver_close(client).await;
        Ok(batch)
    })?;
    let rows = sqlserver_first_rows(&batch, "SQL Server schema 列表返回类型异常")?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let name = row.values.first().and_then(cell_text)?;
            Some(ObjectSummary {
                path: ObjectPath {
                    connection_id: config.id,
                    database: Some(database.clone()),
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
        .collect())
}

fn sqlserver_list_relations(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let database = path
        .database
        .clone()
        .filter(|database| !database.trim().is_empty())
        .ok_or_else(|| Error::new(ErrorKind::Query, "SQL Server 表列表缺少数据库上下文"))?;
    let schema = path
        .schema
        .clone()
        .or_else(|| (!path.name.trim().is_empty()).then(|| path.name.clone()))
        .ok_or_else(|| Error::new(ErrorKind::Query, "SQL Server 表列表缺少 schema 上下文"))?;
    let mut client = sqlserver_connect_database(config, Some(&database))?;
    let batch = sqlserver_block_on(async {
        let batch = sqlserver_run_query(
            &mut client,
            "SELECT o.name, o.type, o.object_id, o.create_date, o.modify_date, \
                    CAST(ep.value AS nvarchar(max)) AS comment, \
                    SUM(CASE WHEN p.index_id IN (0,1) THEN p.rows ELSE 0 END) AS row_count, \
                    SUM(p.used_page_count) * 8192 AS size_bytes \
             FROM sys.objects o \
             JOIN sys.schemas s ON s.schema_id = o.schema_id \
             LEFT JOIN sys.extended_properties ep \
               ON ep.class = 1 AND ep.major_id = o.object_id AND ep.minor_id = 0 AND ep.name = 'MS_Description' \
             LEFT JOIN sys.partitions p ON p.object_id = o.object_id \
             WHERE s.name = @P1 AND o.type IN ('U','V','P','PC','FN','IF','TF','FS','FT') \
               AND o.is_ms_shipped = 0 \
             GROUP BY o.name, o.type, o.object_id, o.create_date, o.modify_date, ep.value \
             ORDER BY o.name",
            &[fluxdb_core::SqlParameter::Text(schema.clone())],
        )
        .await?;
        sqlserver_close(client).await;
        Ok(batch)
    })?;
    let rows = sqlserver_first_rows(&batch, "SQL Server 对象列表返回类型异常")?;
    let mut objects = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(name) = row.values.first().and_then(cell_text) else {
            continue;
        };
        let object_type = row.values.get(1).and_then(cell_text).unwrap_or_default();
        let stable = row.values.get(2).and_then(cell_i64).and_then(|value| u64::try_from(value).ok());
        // `used_page_count` 覆盖堆/聚集索引与非聚集索引；SQL Server 页固定 8 KiB。
        let size_bytes = row
            .values
            .get(7)
            .and_then(cell_i64)
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0);
        objects.push(ObjectSummary {
            path: ObjectPath {
                connection_id: config.id,
                database: Some(database.clone()),
                schema: Some(schema.clone()),
                name,
                kind: match object_type.trim().to_ascii_uppercase().as_str() {
                    "V" => ObjectKind::View,
                    "P" | "PC" => ObjectKind::Procedure,
                    "FN" | "IF" | "TF" | "FS" | "FT" => ObjectKind::Function,
                    _ => ObjectKind::Table,
                },
            },
            rows: row.values.get(6).and_then(cell_i64).and_then(|value| u64::try_from(value).ok()),
            size_bytes,
            modified_at: row.values.get(4).and_then(cell_text),
            comment: row.values.get(5).and_then(cell_text),
            stable,
        });
    }
    Ok(objects)
}

fn sqlserver_first_rows(
    batch: &SqlServerBatchResult,
    message: &str,
) -> fluxdb_core::Result<Vec<Row>> {
    batch
        .result_sets
        .first()
        .map(|set| set.rows.clone())
        .ok_or_else(|| Error::new(ErrorKind::Query, message))
}

fn cell_text(value: &CellValue) -> Option<String> {
    match value {
        CellValue::Text(value) | CellValue::Json(value) => Some(value.clone()),
        CellValue::I64(value) => Some(value.to_string()),
        CellValue::F64(value) => Some(value.to_string()),
        CellValue::Bool(value) => Some(value.to_string()),
        // 预览不是元数据：只取预览会得到被截断的名称/注释。
        CellValue::TruncatedText { .. }
        | CellValue::LargeValue(_)
        | CellValue::Bytes(_)
        | CellValue::BinarySummary(_)
        | CellValue::Null => None,
    }
}

fn cell_i64(value: &CellValue) -> Option<i64> {
    match value {
        CellValue::I64(value) => Some(*value),
        CellValue::Text(value) => value.trim().parse().ok(),
        _ => None,
    }
}
