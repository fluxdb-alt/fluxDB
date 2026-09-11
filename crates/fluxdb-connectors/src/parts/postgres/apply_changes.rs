// PostgreSQL 数据编辑提交 apply_changes（T09）。
//
// 对齐 mysql/sqlite：删 → 更新 → 插入，单事务内执行，任何一步失败整体回滚。
// `pg_connect` 每次拨一条全新连接（非注册表复用），因此在本会话上 `BEGIN…COMMIT`
// 不会与其它请求串事务；语句经 `&Client` 顺序执行，错误时 `ROLLBACK` 兜底。
// PG 不经过 sqlx QueryBuilder：SQL 自行拼接，值一律 `$n` 参数化绑定，标识符 `pg_quote_identifier`。
// 二进制（bytea）写入以 `Vec<u8>` 直接绑定；Hex 编辑回写前由 app 转成 `CellValue::Bytes`。

fn pg_apply_changes(config: &ConnectionConfig, changes: &DataChangeSet) -> fluxdb_core::Result<()> {
    validate_data_changes(changes)?;

    let database = changes
        .object
        .database
        .as_deref()
        .ok_or_else(|| Error::new(ErrorKind::Query, "缺少数据库名称"))?;

    pg_runtime().block_on(async {
        let session = pg_connect(config, database).await?;
        let client = session.client.as_ref();
        let table = pg_qualified_table(database, changes.object.schema.as_deref(), &changes.object.name);
        // 先取列元数据（元数据查询与事务解耦）。
        let columns = pg_columns(client, database, changes.object.schema.as_deref(), &changes.object.name)
            .await?;

        client
            .batch_execute("BEGIN")
            .await
            .map_err(|error| Error::new(ErrorKind::Query, error.to_string()))?;

        let result = async {
            pg_apply_deletes(client, &table, changes, &columns).await?;
            pg_apply_updates(client, &table, changes, &columns).await?;
            pg_apply_inserts(client, &table, changes, &columns).await?;
            Ok(())
        }
        .await;

        match result {
            Ok(()) => client
                .batch_execute("COMMIT")
                .await
                .map_err(|error| Error::new(ErrorKind::Query, error.to_string())),
            Err(error) => {
                let _ = client.batch_execute("ROLLBACK").await;
                Err(error)
            }
        }
    })
}

async fn pg_apply_inserts(
    client: &tokio_postgres::Client,
    table: &str,
    changes: &DataChangeSet,
    columns: &[Column],
) -> fluxdb_core::Result<()> {
    for row in &changes.inserts {
        let insert_values = non_null_insert_values(row, columns)?;
        let (sql, params) = pg_insert_sql(table, &insert_values)?;
        client
            .query_opt(&sql, &pg_params_refs(&params))
            .await
            .map_err(|error| Error::new(ErrorKind::Query, error.to_string()))?;
    }
    Ok(())
}

async fn pg_apply_updates(
    client: &tokio_postgres::Client,
    table: &str,
    changes: &DataChangeSet,
    columns: &[Column],
) -> fluxdb_core::Result<()> {
    for update in &changes.updates {
        if update.cells.is_empty() {
            continue;
        }
        for cell in &update.cells {
            ensure_column_exists(&cell.column, columns)?;
        }
        validate_identity(&update.identity, columns)?;

        let (sql, params) = pg_update_sql(table, &update.cells, &update.identity, columns)?;
        client
            .query_opt(&sql, &pg_params_refs(&params))
            .await
            .map_err(|error| Error::new(ErrorKind::Query, error.to_string()))?;
    }
    Ok(())
}

async fn pg_apply_deletes(
    client: &tokio_postgres::Client,
    table: &str,
    changes: &DataChangeSet,
    columns: &[Column],
) -> fluxdb_core::Result<()> {
    for identity in &changes.deletes {
        validate_identity(identity, columns)?;
        let (where_sql, params) = pg_identity_params(identity, columns)?;
        let sql = format!("DELETE FROM {table}{where_sql}");
        client
            .query_opt(&sql, &pg_params_refs(&params))
            .await
            .map_err(|error| Error::new(ErrorKind::Query, error.to_string()))?;
    }
    Ok(())
}

fn pg_insert_sql(
    table: &str,
    values: &[(&Column, &CellValue)],
) -> fluxdb_core::Result<(String, Vec<Box<dyn ToSql + Sync>>)> {
    if values.is_empty() {
        // 全部字段为 NULL 时退化为默认值插入。
        return Ok((format!("INSERT INTO {table} DEFAULT VALUES"), Vec::new()));
    }

    let mut params: Vec<Box<dyn ToSql + Sync>> = Vec::new();
    let column_list = values
        .iter()
        .map(|(column, _)| pg_quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = values
        .iter()
        .map(|(column, value)| pg_bind_sql(&mut params, column, value))
        .collect::<Vec<_>>()
        .join(", ");
    Ok((
        format!("INSERT INTO {table} ({column_list}) VALUES ({placeholders})"),
        params,
    ))
}

fn pg_update_sql(
    table: &str,
    cells: &[fluxdb_core::CellUpdate],
    identity: &fluxdb_core::RowIdentity,
    columns: &[Column],
) -> fluxdb_core::Result<(String, Vec<Box<dyn ToSql + Sync>>)> {
    let mut params: Vec<Box<dyn ToSql + Sync>> = Vec::new();
    let sets = cells
        .iter()
        .map(|cell| {
            let col = columns
                .iter()
                .find(|column| column.name == cell.column)
                .ok_or_else(|| Error::new(ErrorKind::Query, "更新引用了不存在的列"))?;
            Ok(format!(
                "{} = {}",
                pg_quote_identifier(&cell.column),
                pg_bind_sql(&mut params, col, &cell.value)
            ))
        })
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    // 身份条件继续沿用同一参数序号（紧跟 SET 之后）。
    let where_sql = pg_identity_where(&mut params, identity, columns)?;
    Ok((
        format!("UPDATE {table} SET {sets}{where_sql}"),
        params,
    ))
}

/// 把行身份写成参数化 WHERE 并追加到既有参数表（保证 `$n` 序号连续）。空身份则报错。
fn pg_identity_where(
    params: &mut Vec<Box<dyn ToSql + Sync>>,
    identity: &fluxdb_core::RowIdentity,
    columns: &[Column],
) -> fluxdb_core::Result<String> {
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
            clauses.push(format!("{quoted} = {}", pg_bind_sql(params, col, value)));
        }
    }
    if clauses.is_empty() {
        return Err(Error::new(ErrorKind::Query, "缺少行身份条件，已取消提交"));
    }
    Ok(format!(" WHERE {}", clauses.join(" AND ")))
}
