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
