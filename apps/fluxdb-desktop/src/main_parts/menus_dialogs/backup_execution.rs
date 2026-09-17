/// UI 表单转为应用请求；工具解析和实际执行均由应用层在后台完成。
fn run_backup_task(
    controller: AppController,
    form: BackupForm,
    output: PathBuf,
    cancel: Arc<AtomicBool>,
    sender: mpsc::Sender<BackupTaskProgress>,
) -> anyhow::Result<(PathBuf, fluxdb_app::BackupManifest)> {
    let config = controller
        .connection_configs()
        .into_iter()
        .find(|c| c.id == form.connection_id)
        .ok_or_else(|| anyhow::anyhow!("连接不存在"))?;
    let method = match form.mode {
        BackupMode::Auto => fluxdb_app::BackupMethod::Auto,
    };
    let request = controller
        .prepare_backup(
            fluxdb_app::BackupRequest {
                config,
                database: form.database.unwrap_or_default(),
                output,
                execution: fluxdb_app::BackupExecution::SqlDump,
                tool: PathBuf::new(),
                tool_version: None,
                tables: form.selected_tables.into_iter().collect(),
                include_views: form.include_views,
                include_schema: form.include_schema,
                include_data: form.include_data,
                include_routines: form.include_routines,
                single_transaction: form.single_transaction,
                lock_tables: form.lock_tables,
                include_owner: form.pg_include_owner,
                include_acl: form.pg_include_acl,
            },
            method,
        )
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let _ = sender.send(BackupTaskProgress {
        stage: "文件".into(),
        message: format!(
            "{}：{}",
            request.execution.format().label(),
            request.output.display()
        ),
        success: true,
    });
    let meta = controller
        .execute_backup(&request, &cancel, &mut |event| {
            let _ = sender.send(BackupTaskProgress {
                stage: event.stage,
                message: event.message,
                success: true,
            });
        })
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok((request.output, meta))
}
