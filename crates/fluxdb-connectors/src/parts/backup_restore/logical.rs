fn logical_backup(
    request: &BackupRequest,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(DatabaseTaskProgress),
) -> fluxdb_core::Result<BackupManifest> {
    let connector = connector_for(&request.config)?;
    let root = ObjectPath {
        connection_id: request.config.id,
        database: Some(request.database.clone()),
        schema: None,
        name: String::new(),
        kind: ObjectKind::Schema,
    };
    let objects = connector.list_objects(Some(&root))?;
    let objects: Vec<_> = objects
        .into_iter()
        .filter(|o| match o.path.kind {
            ObjectKind::Table => request.tables.is_empty() || request.tables.contains(&o.path.name),
            ObjectKind::View => request.include_views,
            _ => false,
        })
        .collect();
    let mut writer = std::io::BufWriter::new(new_output(&request.output)?);
    writeln!(writer, "-- fluxDB MySQL logical backup\nSET FOREIGN_KEY_CHECKS=0;\nSET SQL_MODE='NO_BACKSLASH_ESCAPES';").map_err(io_error)?;
    let mut meta = manifest(request);
    meta.objects.clear();
    for object in objects {
        canceled(cancel)?;
        report(progress, "对象", format!("正在导出 {}", object.path.name));
        if request.include_schema {
            writeln!(writer, "{};", connector.table_ddl(&object.path)?).map_err(io_error)?;
        }
        if request.include_data && object.path.kind == ObjectKind::Table {
            let mut offset = 0;
            loop {
                canceled(cancel)?;
                let page = connector.load_data(&object.path, offset, 1000, &[], &[])?;
                for row in &page.rows {
                    let values = row
                        .values
                        .iter()
                        .map(mysql_backup_literal)
                        .collect::<fluxdb_core::Result<Vec<_>>>()?;
                    let columns = page
                        .columns
                        .iter()
                        .map(|c| quote_mysql(&c.name))
                        .collect::<Vec<_>>()
                        .join(",");
                    writeln!(
                        writer,
                        "INSERT INTO {} ({columns}) VALUES ({});",
                        quote_mysql(&object.path.name),
                        values.join(",")
                    )
                    .map_err(io_error)?;
                }
                offset += page.rows.len() as u64;
                if !page.has_more || page.rows.is_empty() {
                    break;
                }
            }
        }
        meta.objects.push(object.path.name);
    }
    writeln!(writer, "SET FOREIGN_KEY_CHECKS=1;").map_err(io_error)?;
    writer.flush().map_err(io_error)?;
    // 任一对象失败直接返回错误，绝不把缺失对象的备份记为完整成功。
    Ok(meta)
}
fn quote_mysql(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
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
                "数据包含未加载的二进制值或非有限浮点数，请改用原生备份",
            ));
        }
    })
}
