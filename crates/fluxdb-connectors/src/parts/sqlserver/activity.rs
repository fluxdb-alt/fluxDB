// SQL Server 服务器活动：DMV 快照、锁等待与终止会话。
//
// 一次读取建一条短连接；锁与状态分区独立报错，避免缺少 VIEW SERVER STATE
// 时整页不可用。SQL Server 没有「只终止查询」的等价命令，查询终止明确拒绝。

fn sqlserver_server_activity(config: &ConnectionConfig) -> fluxdb_core::Result<ServerActivity> {
    let database = sqlserver_database(
        config,
        &sqlserver_profile(config)?,
    );
    let mut client = sqlserver_connect_database(config, database.as_deref())?;
    sqlserver_block_on(async {
        let own_spid = sqlserver_activity_scalar_i64(&mut client, "SELECT @@SPID")
            .await
            .unwrap_or(-1);
        let version = sqlserver_activity_scalar_string(
            &mut client,
            "SELECT CONVERT(varchar(64), SERVERPROPERTY('ProductVersion')) \
                    + ' ' + CONVERT(varchar(64), SERVERPROPERTY('Edition'))",
        )
        .await
        .unwrap_or_default();
        let current_user =
            sqlserver_activity_scalar_string(&mut client, "SELECT SUSER_SNAME()")
                .await
                .unwrap_or_default();
        let sessions = sqlserver_activity_sessions(&mut client, own_spid).await?;
        let (locks, locks_error) = match sqlserver_activity_locks(&mut client, own_spid).await {
            Ok(locks) => (locks, None),
            Err(error) => (Vec::new(), Some(error.message)),
        };
        let (status, status_error) = match sqlserver_activity_status(&mut client).await {
            Ok(status) => (status, None),
            Err(error) => (ServerStatus::default(), Some(error.message)),
        };
        sqlserver_close(client).await;
        Ok(ServerActivity {
            version,
            current_user,
            sessions,
            locks,
            locks_error,
            status,
            status_error,
        })
    })
}

async fn sqlserver_activity_rows(
    client: &mut SqlServerClient,
    sql: &str,
) -> fluxdb_core::Result<Vec<Row>> {
    Ok(sqlserver_run_batch(client, sql)
        .await?
        .result_sets
        .into_iter()
        .next()
        .map(|set| set.rows)
        .unwrap_or_default())
}

async fn sqlserver_activity_scalar_string(
    client: &mut SqlServerClient,
    sql: &str,
) -> fluxdb_core::Result<String> {
    Ok(sqlserver_activity_rows(client, sql)
        .await?
        .first()
        .and_then(|row| row.values.first())
        .map(|value| value.display_label())
        .unwrap_or_default())
}

async fn sqlserver_activity_scalar_i64(
    client: &mut SqlServerClient,
    sql: &str,
) -> fluxdb_core::Result<i64> {
    Ok(sqlserver_activity_rows(client, sql)
        .await?
        .first()
        .and_then(|row| row.values.first())
        .and_then(cell_i64)
        .unwrap_or(0))
}

async fn sqlserver_activity_sessions(
    client: &mut SqlServerClient,
    own_spid: i64,
) -> fluxdb_core::Result<Vec<ServerSession>> {
    let rows = sqlserver_activity_rows(
        client,
        "SELECT s.session_id, \
                COALESCE(s.login_name, ''), \
                COALESCE(s.host_name, '') + CASE \
                    WHEN s.host_process_id IS NULL THEN '' \
                    ELSE ':' + CONVERT(varchar(20), s.host_process_id) END, \
                COALESCE(DB_NAME(s.database_id), ''), \
                COALESCE(r.command, s.status), \
                CASE WHEN COALESCE(r.start_time, s.last_request_start_time, s.login_time) IS NULL \
                    THEN 0 ELSE DATEDIFF(SECOND, COALESCE(r.start_time, s.last_request_start_time, s.login_time), GETDATE()) END, \
                COALESCE(r.wait_type, ''), \
                COALESCE(t.text, ''), \
                COALESCE(s.program_name, '') \
         FROM sys.dm_exec_sessions s \
         LEFT JOIN sys.dm_exec_requests r ON r.session_id = s.session_id \
         OUTER APPLY sys.dm_exec_sql_text(r.sql_handle) t \
         WHERE s.is_user_process = 1 \
         ORDER BY 6 DESC",
    )
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let id = sqlserver_activity_i64(&row, 0);
            let wait_type = sqlserver_activity_text(&row, 6);
            ServerSession {
                id: id.to_string(),
                user: sqlserver_activity_text(&row, 1),
                host: sqlserver_activity_text(&row, 2),
                database: sqlserver_activity_text(&row, 3),
                command: sqlserver_activity_text(&row, 4),
                elapsed_secs: sqlserver_activity_i64(&row, 5).max(0) as u64,
                wait_event: wait_type,
                sql: sqlserver_activity_text(&row, 7),
                is_self: id == own_spid,
            }
        })
        .collect())
}

async fn sqlserver_activity_locks(
    client: &mut SqlServerClient,
    own_spid: i64,
) -> fluxdb_core::Result<Vec<ServerLockWait>> {
    let rows = sqlserver_activity_rows(
        client,
        "SELECT wt.session_id, wt.blocking_session_id, \
                COALESCE(DB_NAME(COALESCE(tl.resource_database_id, DB_ID())), ''), \
                COALESCE(tl.resource_description, wt.resource_description, ''), \
                COALESCE(tl.resource_type, wt.wait_type, ''), \
                COALESCE(tl.request_mode, wt.wait_type, ''), \
                COALESCE(wt.wait_duration_ms / 1000, 0), \
                COALESCE(r.status, 'waiting'), \
                COALESCE(t.text, '') \
         FROM sys.dm_os_waiting_tasks wt \
         JOIN sys.dm_exec_sessions s \
           ON s.session_id = wt.session_id AND s.is_user_process = 1 \
         LEFT JOIN sys.dm_tran_locks tl \
           ON tl.lock_owner_address = wt.resource_address AND tl.request_status = 'WAIT' \
         LEFT JOIN sys.dm_exec_requests r ON r.session_id = wt.session_id \
         OUTER APPLY sys.dm_exec_sql_text(r.sql_handle) t \
         WHERE wt.blocking_session_id IS NOT NULL AND wt.blocking_session_id <> 0 \
           AND wt.session_id <> @@SPID \
         ORDER BY 7 DESC",
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter(|row| sqlserver_activity_i64(row, 0) != own_spid)
        .map(|row| {
            let database = sqlserver_activity_text(&row, 2);
            let object = sqlserver_activity_text(&row, 3);
            ServerLockWait {
                waiting_session: sqlserver_activity_i64(&row, 0).to_string(),
                blocking_session: sqlserver_activity_i64(&row, 1).to_string(),
                object: match (database.is_empty(), object.is_empty()) {
                    (true, _) => object,
                    (false, true) => database,
                    (false, false) => format!("{database}.{object}"),
                },
                lock_type: sqlserver_activity_text(&row, 4),
                wait_secs: sqlserver_activity_i64(&row, 6).max(0) as u64,
                state: sqlserver_activity_text(&row, 7),
                sql: sqlserver_activity_text(&row, 8),
            }
        })
        .collect())
}

async fn sqlserver_activity_status(
    client: &mut SqlServerClient,
) -> fluxdb_core::Result<ServerStatus> {
    let rows = sqlserver_activity_rows(
        client,
        "SELECT (SELECT COUNT(*) FROM sys.dm_exec_sessions WHERE is_user_process = 1), \
                (SELECT COUNT(*) FROM sys.dm_exec_requests r \
                 JOIN sys.dm_exec_sessions s ON s.session_id = r.session_id \
                 WHERE s.is_user_process = 1 AND r.session_id <> @@SPID), \
                (SELECT COUNT(*) FROM sys.dm_os_waiting_tasks wt \
                 JOIN sys.dm_exec_sessions s ON s.session_id = wt.session_id \
                 WHERE s.is_user_process = 1 \
                   AND wt.blocking_session_id IS NOT NULL AND wt.blocking_session_id <> 0), \
                DATEDIFF(SECOND, sqlserver_start_time, GETDATE()), \
                cpu_count, physical_memory_kb \
         FROM sys.dm_os_sys_info",
    )
    .await?;
    let Some(row) = rows.first() else {
        return Ok(ServerStatus::default());
    };
    Ok(ServerStatus {
        uptime_secs: Some(sqlserver_activity_i64(row, 3).max(0) as u64),
        metrics: vec![
            ServerMetric {
                key: "sessions".into(),
                value: sqlserver_activity_i64(row, 0).to_string(),
                warning: false,
            },
            ServerMetric {
                key: "active_requests".into(),
                value: sqlserver_activity_i64(row, 1).to_string(),
                warning: false,
            },
            ServerMetric {
                key: "lock_waits".into(),
                value: sqlserver_activity_i64(row, 2).to_string(),
                warning: true,
            },
            ServerMetric {
                key: "cpu_count".into(),
                value: sqlserver_activity_i64(row, 4).to_string(),
                warning: false,
            },
            ServerMetric {
                key: "physical_memory_kb".into(),
                value: sqlserver_activity_i64(row, 5).to_string(),
                warning: false,
            },
        ],
    })
}

fn sqlserver_activity_text(row: &Row, index: usize) -> String {
    row.values
        .get(index)
        .map(|value| match value {
            CellValue::Null => String::new(),
            other => other.display_label(),
        })
        .unwrap_or_default()
}

fn sqlserver_activity_i64(row: &Row, index: usize) -> i64 {
    row.values.get(index).and_then(cell_i64).unwrap_or(0)
}

fn sqlserver_kill(config: &ConnectionConfig, session_id: &str, query_only: bool) -> fluxdb_core::Result<()> {
    if config.is_read_only() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "只读连接不允许终止查询或会话",
        ));
    }
    if query_only {
        // SQL Server 只提供会话级 KILL；把它伪装成「取消查询」会悄悄断开连接。
        return Err(Error::new(
            ErrorKind::Unsupported,
            "SQL Server 不支持仅终止查询，请终止整个会话",
        ));
    }
    let target = session_id
        .trim()
        .parse::<i64>()
        .map_err(|_| Error::new(ErrorKind::Query, "SQL Server 会话 ID 必须是数字 spid"))?;
    let mut client = sqlserver_connect(config)?;
    sqlserver_block_on(async {
        let own_spid = sqlserver_activity_scalar_i64(&mut client, "SELECT @@SPID").await?;
        if own_spid == target {
            return Err(Error::new(
                ErrorKind::Query,
                "不能终止服务器面板自身使用的会话",
            ));
        }
        let result = sqlserver_run_batch(&mut client, &format!("KILL {target}")).await;
        sqlserver_close(client).await;
        result.map(|_| ())
    })
}

fn sqlserver_server_maintenance(
    config: &ConnectionConfig,
    request: &ServerMaintenanceRequest,
) -> fluxdb_core::Result<String> {
    if config.is_read_only() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "只读连接不允许执行维护",
        ));
    }
    let database = if request.database.trim().is_empty() {
        sqlserver_request_database(config, None)?
    } else {
        request.database.trim().to_string()
    };
    let statement = match request.action {
        // 等价于「更新当前库全部统计信息」。
        ServerMaintenanceAction::Analyze => "EXEC sp_updatestats".to_string(),
        // 重建当前库所有用户表的全部索引；逐表执行，单表失败即中止。
        ServerMaintenanceAction::Reindex => "DECLARE @sql nvarchar(max) = N''; \
             SELECT @sql += N'ALTER INDEX ALL ON ' + QUOTENAME(SCHEMA_NAME(t.schema_id)) \
                 + N'.' + QUOTENAME(t.name) + N' REBUILD;' \
             FROM sys.tables t; \
             EXEC sp_executesql @sql;"
            .to_string(),
        ServerMaintenanceAction::Vacuum | ServerMaintenanceAction::Optimize => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "该维护动作不适用于 SQL Server",
            ));
        }
    };
    let mut client = sqlserver_connect_database(config, Some(&database))?;
    sqlserver_block_on(async {
        let result = sqlserver_run_batch(&mut client, &statement).await;
        sqlserver_close(client).await;
        result.map(|_| String::new())
    })
}
