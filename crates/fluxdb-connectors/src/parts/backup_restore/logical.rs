fn logical_backup(
    request: &BackupRequest,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(DatabaseTaskProgress),
) -> fluxdb_core::Result<BackupManifest> {
    // 执行前解析成固定快照；空选择已被 resolve_backup_scope 拒绝。
    let resolved = resolve_backup_scope(request)?;
    let connector = connector_for(&request.config)?;
    let root = ObjectPath {
        connection_id: request.config.id,
        database: Some(request.database.clone()),
        schema: None,
        name: String::new(),
        kind: ObjectKind::Schema,
    };
    let objects = connector.list_objects(Some(&root))?;
    let selected: std::collections::BTreeSet<&str> =
        resolved.keys.iter().map(String::as_str).collect();
    let objects: Vec<_> = objects
        .into_iter()
        .filter(|o| {
            matches!(o.path.kind, ObjectKind::Table | ObjectKind::View)
                && selected.contains(o.path.name.as_str())
        })
        .collect();
    // 用户要求的对象在执行前被删除：报错列出缺失项，不静默缩小范围后宣称完整成功。
    if objects.len() != resolved.keys.len() {
        let found: std::collections::BTreeSet<&str> =
            objects.iter().map(|o| o.path.name.as_str()).collect();
        let missing: Vec<&str> = resolved
            .keys
            .iter()
            .map(String::as_str)
            .filter(|key| !found.contains(key))
            .collect();
        return Err(task_error(format!(
            "所选对象在数据库中不存在：{}",
            missing.join("、")
        )));
    }
    let mut writer = std::io::BufWriter::new(new_output(&request.output)?);
    writeln!(writer, "-- fluxDB MySQL logical backup\nSET FOREIGN_KEY_CHECKS=0;\nSET SQL_MODE='NO_BACKSLASH_ESCAPES';").map_err(io_error)?;
    let mut meta = manifest(request, Vec::new());
    // 优先单连接一致快照（DDL 与数据同一视图）；不支持时退回逐页读取并标注非一致。
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
    for object in objects {
        canceled(cancel)?;
        report(progress, "对象", format!("正在导出 {}", object.path.name));
        if request.include_schema {
            let ddl = match snapshot.as_mut() {
                Some(snapshot) => snapshot.table_ddl(&object.path)?,
                None => connector.table_ddl(&object.path)?,
            };
            writeln!(writer, "{ddl};").map_err(io_error)?;
        }
        let mut rows_exported = 0u64;
        if request.include_data && object.path.kind == ObjectKind::Table {
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
                    match snapshot.load_data_keyset(&object.path, &cursor, 1000) {
                        Ok(page) => (page, true),
                        Err(error) if error.kind == fluxdb_core::ErrorKind::Unsupported => {
                            keyset = false;
                            fallback_reported = true;
                            report(
                                progress,
                                "范围",
                                format!(
                                    "{} 无主键，改用 OFFSET 分页，并发写入时可能重复或漏行",
                                    object.path.name
                                ),
                            );
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    let page = match snapshot.as_mut() {
                        Some(snapshot) => snapshot.load_data(&object.path, offset, 1000)?,
                        None => connector.load_data(&object.path, offset, 1000, &[], &[])?,
                    };
                    (page, false)
                };
                has_primary_key &= page.columns.iter().any(|column| column.primary_key);
                write_mysql_page(&mut writer, &object.path.name, &page)?;
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
            // 无主键时排序没有稳定 tie breaker，跨页 OFFSET 在并发写入下可能重复或漏行。
            if pages > 1 && !has_primary_key && !fallback_reported {
                report(
                    progress,
                    "范围",
                    format!(
                        "{} 无主键，分页读取不保证稳定顺序，并发写入时可能重复或漏行",
                        object.path.name
                    ),
                );
            }
        }
        meta.objects.push(object.path.name.clone());
        meta.object_stats.push(BackupObjectStats {
            name: object.path.name.clone(),
            // 视图不导出数据，行数记为 None；恢复校验只比对有值对象。
            rows: (request.include_data && object.path.kind == ObjectKind::Table)
                .then_some(rows_exported),
        });
    }
    // 快照状态写进清单：false 表示本次是逐页独立读取，不能宣称为一致快照。
    meta.consistent = Some(snapshot.is_some());
    if let Some(mut snapshot) = snapshot.take() {
        meta.snapshot_id = snapshot.snapshot_id();
        snapshot.finish()?;
    }
    writeln!(writer, "SET FOREIGN_KEY_CHECKS=1;").map_err(io_error)?;
    writer.flush().map_err(io_error)?;
    drop(writer);
    // 追加内嵌清单：footer 在数据写完后生成，因此可携带整文件 sha256 与逐表行数。
    finalize_embedded_manifest(&request.output, &mut meta)?;
    // 任一对象失败直接返回错误，绝不把缺失对象的备份记为完整成功。
    Ok(meta)
}

/// 单条 INSERT 携带的最大行数；兼顾文件体积与单语句大小（大字段场景不拼接整页）。
const BACKUP_INSERT_BATCH_ROWS: usize = 100;

/// 从一页的最后一行提取主键游标；无主键或行缺失时返回 None。
///
/// 顺序与 `DataPage.columns` 中 `primary_key` 列一致，正好是
/// `BackupSnapshot::load_data_keyset` 期望的 `after` 参数。
fn backup_cursor(page: &fluxdb_core::DataPage) -> Option<Vec<CellValue>> {
    let last = page.rows.last()?;
    let cursor = page
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.primary_key)
        .map(|(index, _)| last.values.get(index).cloned())
        .collect::<Option<Vec<_>>>()?;
    (!cursor.is_empty()).then_some(cursor)
}

/// 把一页行写成多行 INSERT；列名取本页投影结果，保证与行值顺序一致。
fn write_mysql_page(
    writer: &mut impl Write,
    table: &str,
    page: &fluxdb_core::DataPage,
) -> fluxdb_core::Result<()> {
    let columns = page
        .columns
        .iter()
        .map(|column| quote_mysql(&column.name))
        .collect::<Vec<_>>()
        .join(",");
    for rows in page.rows.chunks(BACKUP_INSERT_BATCH_ROWS) {
        let tuples = rows
            .iter()
            .map(|row| {
                let values = row
                    .values
                    .iter()
                    .map(mysql_backup_literal)
                    .collect::<fluxdb_core::Result<Vec<_>>>()?;
                Ok(format!("({})", values.join(",")))
            })
            .collect::<fluxdb_core::Result<Vec<_>>>()?;
        writeln!(
            writer,
            "INSERT INTO {} ({columns}) VALUES {};",
            quote_mysql(table),
            tuples.join(",")
        )
        .map_err(io_error)?;
    }
    Ok(())
}
fn quote_mysql(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}
#[cfg(test)]
mod mysql_write_tests {
    use super::*;

    fn page(count: usize) -> fluxdb_core::DataPage {
        fluxdb_core::DataPage {
            columns: vec![fluxdb_core::Column {
                name: "name".into(),
                type_name: Some("varchar".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            }],
            rows: (0..count)
                .map(|index| fluxdb_core::Row {
                    values: vec![CellValue::Text(format!("v{index}"))],
                })
                .collect(),
            offset: 0,
            limit: 1000,
            has_more: false,
            incomplete: None,
        }
    }

    #[test]
    fn mysql_page_batches_rows_and_escapes_values() {
        let mut output = Vec::new();
        write_mysql_page(&mut output, "t", &page(101)).expect("write");
        let sql = String::from_utf8(output).expect("utf8");
        // 101 行拆成 100 + 1 两条 INSERT，避免逐行一条导致文件膨胀。
        assert_eq!(sql.matches("INSERT INTO").count(), 2);
        assert!(sql.contains("('v0'),"));
        assert!(sql.trim_end().ends_with("('v100');"));
    }
}

fn mysql_backup_literal(value: &CellValue) -> fluxdb_core::Result<String> {
    Ok(match value {
        CellValue::Null => "NULL".into(),
        CellValue::Bool(v) => if *v { "1" } else { "0" }.into(),
        CellValue::I64(v) => v.to_string(),
        CellValue::F64(v) if v.is_finite() => v.to_string(),
        CellValue::Text(v) | CellValue::Json(v) => format!("'{}'", v.replace('\'', "''")),
        CellValue::Bytes(v) => format!(
            "X'{}'",
            v.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ),
        CellValue::BinarySummary(s) if s.is_null => "NULL".into(),
        _ => {
            return Err(task_error(
                "数据包含未加载的二进制值、截断的 LOB 或非有限浮点数，请改用原生备份",
            ));
        }
    })
}
