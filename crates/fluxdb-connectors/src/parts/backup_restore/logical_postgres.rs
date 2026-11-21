/// PostgreSQL 自研逻辑备份（无 pg_dump 或用户显式选逻辑方式时使用）。
///
/// 与 MySQL 逻辑备份同构：优先在单连接只读 `REPEATABLE READ` 快照上读取 DDL 与数据，
/// 不支持时退回逐页独立读取并在 manifest 标 `consistent=false`。
/// 输出面向 `psql`：只含可静态执行的 DDL 与 INSERT，不依赖 psql 元命令。
fn logical_backup_postgres(
    request: &BackupRequest,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(DatabaseTaskProgress),
) -> fluxdb_core::Result<BackupManifest> {
    // 执行前把动态范围解析成固定快照；空选择已被 resolve_backup_scope 拒绝。
    let resolved = resolve_backup_scope(request)?;
    if resolved.objects.is_empty() {
        return Err(task_error("PostgreSQL 逻辑备份至少需要一个对象"));
    }
    let connector = connector_for(&request.config)?;
    let mut writer = std::io::BufWriter::new(new_output(&request.output)?);
    writeln!(writer, "-- fluxDB PostgreSQL logical backup").map_err(io_error)?;
    // 固定会话参数：脚本可脱离 pg_dump 环境独立执行，反斜杠按字面处理。
    writeln!(
        writer,
        "SET client_encoding = 'UTF8';\nSET standard_conforming_strings = on;\nSET statement_timeout = 0;\nSET client_min_messages = warning;"
    )
    .map_err(io_error)?;
    let mut meta = manifest(request, Vec::new());
    let mut snapshot = match connector.backup_snapshot(&request.database) {
        Ok(snapshot) => Some(snapshot),
        Err(error) if error.kind == fluxdb_core::ErrorKind::Unsupported => {
            report(
                progress,
                "范围",
                "当前数据库不支持一致快照，跨对象读取期间的数据变化可能混入备份",
            );
            None
        }
        Err(error) => return Err(error),
    };
    for object in &resolved.objects {
        canceled(cancel)?;
        let key = object.key();
        let path = ObjectPath {
            connection_id: request.config.id,
            database: Some(request.database.clone()),
            schema: object.schema.clone(),
            name: object.name.clone(),
            kind: match object.kind {
                BackupObjectKind::View => ObjectKind::View,
                _ => ObjectKind::Table,
            },
        };
        report(progress, "对象", format!("正在导出 {key}"));
        if request.include_schema {
            let ddl = match snapshot.as_mut() {
                Some(snapshot) => snapshot.table_ddl(&path)?,
                None => connector.table_ddl(&path)?,
            };
            writeln!(writer, "{};", ddl.trim_end_matches(';')).map_err(io_error)?;
        }
        let mut rows_exported = 0u64;
        if request.include_data && object.kind == BackupObjectKind::Table {
            let mut offset = 0;
            let mut pages = 0u64;
            let mut has_primary_key = true;
            // 有快照时优先主键键集分页；无主键或引擎不支持时自动退回 OFFSET。
            let mut keyset = snapshot.is_some();
            let mut cursor: Vec<CellValue> = Vec::new();
            let mut fallback_reported = false;
            loop {
                canceled(cancel)?;
                let (page, used_keyset) = if keyset {
                    let snapshot = snapshot.as_mut().expect("keyset 需要快照");
                    match snapshot.load_data_keyset(&path, &cursor, 1000) {
                        Ok(page) => (page, true),
                        Err(error) if error.kind == fluxdb_core::ErrorKind::Unsupported => {
                            keyset = false;
                            fallback_reported = true;
                            report(
                                progress,
                                "范围",
                                format!(
                                    "{key} 无主键，改用 OFFSET 分页，并发写入时可能重复或漏行"
                                ),
                            );
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    let page = match snapshot.as_mut() {
                        Some(snapshot) => snapshot.load_data(&path, offset, 1000)?,
                        None => connector.load_data(&path, offset, 1000, &[], &[])?,
                    };
                    (page, false)
                };
                has_primary_key &= page.columns.iter().any(|column| column.primary_key);
                write_postgres_page(&mut writer, object, &page)?;
                rows_exported += page.rows.len() as u64;
                offset += page.rows.len() as u64;
                pages += 1;
                if used_keyset {
                    cursor = backup_cursor(&page).unwrap_or_default();
                }
                if !page.has_more || page.rows.is_empty() {
                    break;
                }
            }
            if pages > 1 && !has_primary_key && !fallback_reported {
                report(
                    progress,
                    "范围",
                    format!("{key} 无主键，分页读取不保证稳定顺序，并发写入时可能重复或漏行"),
                );
            }
        }
        meta.objects.push(key.clone());
        meta.object_stats.push(BackupObjectStats {
            name: key,
            rows: (request.include_data && object.kind == BackupObjectKind::Table)
                .then_some(rows_exported),
        });
    }
    meta.consistent = Some(snapshot.is_some());
    if let Some(mut snapshot) = snapshot.take() {
        meta.snapshot_id = snapshot.snapshot_id();
        snapshot.finish()?;
    }
    writer.flush().map_err(io_error)?;
    drop(writer);
    finalize_embedded_manifest(&request.output, &mut meta)?;
    Ok(meta)
}

/// 把一页行写成多行 INSERT；列名取本页投影结果，保证与行值顺序一致。
fn write_postgres_page(
    writer: &mut impl Write,
    object: &BackupObjectRef,
    page: &fluxdb_core::DataPage,
) -> fluxdb_core::Result<()> {
    let table = postgres_backup_object_name(object);
    let columns = page
        .columns
        .iter()
        .map(|column| fluxdb_core::PostgresDialect.quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    for rows in page.rows.chunks(BACKUP_INSERT_BATCH_ROWS) {
        let tuples = rows
            .iter()
            .map(|row| {
                let values = row
                    .values
                    .iter()
                    .map(postgres_backup_literal)
                    .collect::<fluxdb_core::Result<Vec<_>>>()?;
                Ok(format!("({})", values.join(", ")))
            })
            .collect::<fluxdb_core::Result<Vec<_>>>()?;
        writeln!(
            writer,
            "INSERT INTO {table} ({columns}) VALUES {};",
            tuples.join(", ")
        )
        .map_err(io_error)?;
    }
    Ok(())
}

fn postgres_backup_object_name(object: &BackupObjectRef) -> String {
    let schema = object.schema.as_deref().unwrap_or("public");
    format!(
        "{}.{}",
        fluxdb_core::PostgresDialect.quote_identifier(schema),
        fluxdb_core::PostgresDialect.quote_identifier(&object.name)
    )
}

#[cfg(test)]
mod postgres_write_tests {
    use super::*;

    fn page(count: usize) -> fluxdb_core::DataPage {
        fluxdb_core::DataPage {
            columns: vec![fluxdb_core::Column {
                name: "id".into(),
                type_name: Some("integer".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            }],
            rows: (0..count)
                .map(|index| fluxdb_core::Row {
                    values: vec![CellValue::I64(index as i64)],
                })
                .collect(),
            offset: 0,
            limit: 1000,
            has_more: false,
            incomplete: None,
        }
    }

    #[test]
    fn postgres_page_batches_rows() {
        let object = BackupObjectRef {
            schema: Some("public".into()),
            kind: BackupObjectKind::Table,
            name: "t".into(),
        };
        let mut output = Vec::new();
        write_postgres_page(&mut output, &object, &page(101)).expect("write");
        let sql = String::from_utf8(output).expect("utf8");
        assert_eq!(sql.matches("INSERT INTO").count(), 2);
        assert!(sql.contains("(0), (1),"));
        assert!(sql.trim_end().ends_with("(100);"));
    }
}

/// 生成 PostgreSQL 字面量。
///
/// 文本只把 `'` 双写：文件头固定 `standard_conforming_strings = on`，
/// 反斜杠按字面值处理，不需要也不会与 `E''` 转义混淆。
/// 二进制统一输出 `'\x...'::bytea`，与非有限浮点/未加载值一样拒绝而非降级。
fn postgres_backup_literal(value: &CellValue) -> fluxdb_core::Result<String> {
    Ok(match value {
        CellValue::Null => "NULL".into(),
        CellValue::Bool(value) => if *value { "TRUE" } else { "FALSE" }.into(),
        CellValue::I64(value) => value.to_string(),
        CellValue::F64(value) if value.is_finite() => value.to_string(),
        CellValue::Text(value) | CellValue::Json(value) => {
            format!("'{}'", value.replace('\'', "''"))
        }
        CellValue::Bytes(value) => format!(
            "'\\x{}'::bytea",
            value.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ),
        CellValue::BinarySummary(summary) if summary.is_null => "NULL".into(),
        _ => {
            return Err(task_error(
                "数据包含未加载的二进制值、截断的大字段或非有限浮点数，请改用原生备份",
            ));
        }
    })
}
