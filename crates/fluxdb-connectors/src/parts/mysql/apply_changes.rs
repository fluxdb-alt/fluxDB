fn mysql_apply_changes(config: &ConnectionConfig, changes: &DataChangeSet) -> fluxdb_core::Result<()> {
    validate_data_changes(changes)?;

    let database = mysql_database_for_path(config, &changes.object)?;
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = plan.connect().await?;
        let result = async {
            let columns = mysql_columns(&mut connection, &database, &changes.object.name).await?;
            let mut transaction = connection.begin().await.map_err(mysql_error)?;
            mysql_apply_deletes(&mut transaction, &database, changes, &columns).await?;
            mysql_apply_updates(&mut transaction, &database, changes, &columns).await?;
            mysql_apply_inserts(&mut transaction, &database, changes, &columns).await?;
            transaction.commit().await.map_err(|error| {
                // 服务端回了错误报文 = 事务已被终止，结果是确定的失败；
                // 没有报文（IO/连接断开）= 提交是否生效未知，必须让调用方核实。
                let unknown = !matches!(error, sqlx::Error::Database(_));
                let error = mysql_error(error);
                if unknown {
                    error.mark_outcome_unknown("提交 MySQL 事务")
                } else {
                    error
                }
            })?;
            Ok(())
        }
        .await;
        connection.close().await.map_err(mysql_error)?;
        result
    })
}

async fn mysql_apply_inserts(
    connection: &mut sqlx::Transaction<'_, MySql>,
    database: &str,
    changes: &DataChangeSet,
    columns: &[Column],
) -> fluxdb_core::Result<()> {
    let table_name = mysql_qualified_table(database, &changes.object.name);
    for row in &changes.inserts {
        let insert_values = non_null_insert_values(row, columns)?;
        if insert_values.is_empty() {
            sqlx::query(&format!("INSERT INTO {table_name} () VALUES ()"))
                .execute(&mut **connection)
                .await
                .map_err(mysql_error)?;
            continue;
        }

        let mut builder = QueryBuilder::<MySql>::new(format!("INSERT INTO {table_name} ("));
        push_insert_column_list(&mut builder, &insert_values, |value| fluxdb_core::MySqlDialect.quote_identifier(value));
        builder.push(") VALUES (");
        for (index, (_, value)) in insert_values.iter().enumerate() {
            if index > 0 {
                builder.push(", ");
            }
            push_mysql_bind(&mut builder, *value);
        }
        builder.push(")");
        builder
            .build()
            .execute(&mut **connection)
            .await
            .map_err(mysql_error)?;
    }

    Ok(())
}

async fn mysql_apply_updates(
    connection: &mut sqlx::Transaction<'_, MySql>,
    database: &str,
    changes: &DataChangeSet,
    columns: &[Column],
) -> fluxdb_core::Result<()> {
    let table_name = mysql_qualified_table(database, &changes.object.name);
    for update in &changes.updates {
        if update.cells.is_empty() {
            continue;
        }
        validate_identity(&update.identity, columns)?;
        for cell in &update.cells {
            ensure_column_exists(&cell.column, columns)?;
        }

        let (plain, streamed) = split_update_cells(update);
        if !plain.is_empty() {
            let mut builder = QueryBuilder::<MySql>::new(format!("UPDATE {table_name} SET "));
            for (index, cell) in plain.iter().enumerate() {
                if index > 0 {
                    builder.push(", ");
                }
                builder
                    .push(fluxdb_core::MySqlDialect.quote_identifier(&cell.column))
                    .push(" = ");
                push_mysql_bind(&mut builder, cell.value.placeholder());
            }
            push_mysql_identity_where(&mut builder, &update.identity, columns)?;
            builder
                .build()
                .execute(&mut **connection)
                .await
                .map_err(mysql_error)?;
        }

        // 流式大字段：先把值清成空串再按块追加，每块一条 UPDATE，进程内存只占一块。
        //
        // 用空串而不是 NULL 作为起点：目标列可能 NOT NULL，写 NULL 会直接失败；
        // 空串对 BLOB 与 TEXT 都合法，且 `CONCAT(IFNULL(c,''), ?)` 对两者都成立。
        // 代价：每次追加都会重写整值（O(n²)），见 `BLOB_STREAM_CHUNK_BYTES`。
        // 文本与二进制共用 `CONCAT(IFNULL(c,''), ?)`：MySQL 对 BLOB 列接二进制参数、
        // 对 TEXT/CHAR 列接文本参数都能正确拼接，分块边界已由 `streamed_chunk_at`
        // 对齐到 UTF-8 字符边界（文本），不会写进半截字符。
        for (cell, stream) in streamed {
            let quoted = fluxdb_core::MySqlDialect.quote_identifier(&cell.column);
            let mut set_null =
                QueryBuilder::<MySql>::new(format!("UPDATE {table_name} SET {quoted} = ''"));
            push_mysql_identity_where(&mut set_null, &update.identity, columns)?;
            let affected = set_null
                .build()
                .execute(&mut **connection)
                .await
                .map_err(mysql_error)?
                .rows_affected();
            require_single_streamed_row(affected, &cell.column)?;

            let mut offset = 0u64;
            while let Some(bytes) = streamed_chunk_at(stream, offset)? {
                offset += bytes.len() as u64;
                let mut builder = QueryBuilder::<MySql>::new(format!(
                    "UPDATE {table_name} SET {quoted} = CONCAT(IFNULL({quoted}, ''), "
                ));
                builder.push_bind(bytes);
                builder.push(")");
                // 身份条件在同一条语句里，避免「先按身份选中、再全表追加」的窗口。
                push_mysql_identity_where(&mut builder, &update.identity, columns)?;
                builder
                    .build()
                    .execute(&mut **connection)
                    .await
                    .map_err(mysql_error)?
                    .rows_affected()
                    .eq(&1)
                    .then_some(())
                    .ok_or_else(|| streamed_row_lost_error(&cell.column))?;
            }
        }
    }

    Ok(())
}

async fn mysql_apply_deletes(
    connection: &mut sqlx::Transaction<'_, MySql>,
    database: &str,
    changes: &DataChangeSet,
    columns: &[Column],
) -> fluxdb_core::Result<()> {
    let table_name = mysql_qualified_table(database, &changes.object.name);
    for identity in &changes.deletes {
        validate_identity(identity, columns)?;

        let mut builder = QueryBuilder::<MySql>::new(format!("DELETE FROM {table_name}"));
        push_mysql_identity_where(&mut builder, identity, columns)?;
        builder
            .build()
            .execute(&mut **connection)
            .await
            .map_err(mysql_error)?;
    }

    Ok(())
}

