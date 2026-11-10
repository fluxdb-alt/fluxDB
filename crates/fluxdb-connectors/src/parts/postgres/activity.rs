// PostgreSQL 服务器活动：pg_stat_activity + pg_locks + pg_blocking_pids。
//
// 一次读取建一条短连接；锁与状态分区独立报错，避免某个视图权限不足时整页失败。
// 终止会话调用 pg_cancel_backend / pg_terminate_backend，并拒绝操作面板自身连接。

fn pg_server_activity(config: &ConnectionConfig) -> fluxdb_core::Result<ServerActivity> {
    let database = pg_request_database(config, None);
    pg_runtime().block_on(async {
        let session = pg_connect(config, &database).await?;
        let client = session.client.as_ref();
        let version = pg_query_scalar_string(client, "SELECT current_setting('server_version')")
            .await
            .unwrap_or_default();
        let current_user = pg_query_scalar_string(client, "SELECT current_user")
            .await
            .unwrap_or_default();
        let own_pid = client
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .map(|row| row.get::<_, i32>(0))
            .ok();

        let sessions = pg_read_sessions(client, own_pid).await?;
        let (locks, locks_error) = match pg_read_lock_waits(client).await {
            Ok(locks) => (locks, None),
            Err(error) => (Vec::new(), Some(error.message)),
        };
        let (status, status_error) = match pg_read_status(client).await {
            Ok(status) => (status, None),
            Err(error) => (ServerStatus::default(), Some(error.message)),
        };

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

async fn pg_query_scalar_string(
    client: &tokio_postgres::Client,
    sql: &str,
) -> fluxdb_core::Result<String> {
    client
        .query_one(sql, &[])
        .await
        .map(|row| row.get::<_, String>(0))
        .map_err(pg_error)
}

async fn pg_read_sessions(
    client: &tokio_postgres::Client,
    own_pid: Option<i32>,
) -> fluxdb_core::Result<Vec<ServerSession>> {
    let rows = client
        .query(
            "SELECT pid, \
                    COALESCE(usename, '') AS user_name, \
                    COALESCE(client_addr::text, '') AS client_addr, \
                    COALESCE(client_port, 0) AS client_port, \
                    COALESCE(datname, '') AS datname, \
                    COALESCE(state, '') AS state, \
                    GREATEST(COALESCE(EXTRACT(EPOCH FROM (now() - COALESCE(query_start, backend_start)))::bigint, 0), 0) AS elapsed_secs, \
                    COALESCE(wait_event_type, '') AS wait_event_type, \
                    COALESCE(wait_event, '') AS wait_event, \
                    COALESCE(query, '') AS query \
             FROM pg_stat_activity \
             ORDER BY elapsed_secs DESC \
             LIMIT 500",
            &[],
        )
        .await
        .map_err(pg_error)?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let pid = row.get::<_, i32>(0);
            let client_addr = row.get::<_, String>(2);
            let client_port = row.get::<_, i32>(3);
            let host = if client_addr.is_empty() {
                String::new()
            } else if client_port > 0 {
                format!("{client_addr}:{client_port}")
            } else {
                client_addr
            };
            let wait_event_type = row.get::<_, String>(7);
            let wait_event = row.get::<_, String>(8);
            let wait = match (wait_event_type.is_empty(), wait_event.is_empty()) {
                (true, true) => String::new(),
                (true, false) => wait_event,
                (false, true) => wait_event_type,
                (false, false) => format!("{wait_event_type}:{wait_event}"),
            };
            ServerSession {
                id: pid.to_string(),
                user: row.get::<_, String>(1),
                host,
                database: row.get::<_, String>(4),
                command: row.get::<_, String>(5),
                elapsed_secs: row.get::<_, i64>(6).max(0) as u64,
                wait_event: wait,
                sql: row.get::<_, String>(9),
                is_self: own_pid == Some(pid),
            }
        })
        .collect())
}

async fn pg_read_lock_waits(
    client: &tokio_postgres::Client,
) -> fluxdb_core::Result<Vec<ServerLockWait>> {
    let rows = client
        .query(
            "SELECT w.pid, \
                    b.pid, \
                    COALESCE(n.nspname, '') AS schema_name, \
                    COALESCE(c.relname, '') AS rel_name, \
                    COALESCE(l.locktype, '') AS locktype, \
                    COALESCE(l.mode, '') AS mode, \
                    GREATEST(COALESCE(EXTRACT(EPOCH FROM (now() - COALESCE(w.query_start, w.backend_start)))::bigint, 0), 0) AS wait_secs, \
                    COALESCE(w.state, '') AS state, \
                    COALESCE(w.query, '') AS query \
             FROM pg_stat_activity w \
             CROSS JOIN LATERAL unnest(pg_blocking_pids(w.pid)) AS bp(blocking_pid) \
             JOIN pg_stat_activity b ON b.pid = bp.blocking_pid \
             LEFT JOIN pg_locks l ON l.pid = w.pid AND NOT l.granted \
             LEFT JOIN pg_class c ON c.oid = l.relation \
             LEFT JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE w.pid <> pg_backend_pid() \
             ORDER BY wait_secs DESC \
             LIMIT 500",
            &[],
        )
        .await
        .map_err(pg_error)?;

    let mut seen = std::collections::BTreeSet::new();
    let mut locks = Vec::new();
    for row in rows {
        let waiting = row.get::<_, i32>(0).to_string();
        let blocking = row.get::<_, i32>(1).to_string();
        let schema_name = row.get::<_, String>(2);
        let rel_name = row.get::<_, String>(3);
        let object = if rel_name.is_empty() {
            String::new()
        } else if schema_name.is_empty() {
            rel_name
        } else {
            format!("{schema_name}.{rel_name}")
        };
        let lock_type = row.get::<_, String>(4);
        if !seen.insert((waiting.clone(), blocking.clone(), object.clone(), lock_type.clone())) {
            continue;
        }
        locks.push(ServerLockWait {
            waiting_session: waiting,
            blocking_session: blocking,
            object,
            lock_type,
            wait_secs: row.get::<_, i64>(6).max(0) as u64,
            state: "waiting".to_string(),
            sql: row.get::<_, String>(8),
        });
    }
    Ok(locks)
}

async fn pg_read_status(
    client: &tokio_postgres::Client,
) -> fluxdb_core::Result<ServerStatus> {
    let uptime_secs = client
        .query_one(
            "SELECT GREATEST(EXTRACT(EPOCH FROM (now() - pg_postmaster_start_time()))::bigint, 0)",
            &[],
        )
        .await
        .map(|row| row.get::<_, i64>(0).max(0) as u64)
        .ok();

    let mut metrics = Vec::new();
    if let Ok(row) = client
        .query_one(
            "SELECT numbackends, xact_commit, blks_hit, blks_read, deadlocks, \
                    COALESCE(temp_bytes, 0), tup_returned, tup_fetched \
             FROM pg_stat_database WHERE datname = current_database()",
            &[],
        )
        .await
    {
        metrics.push(ServerMetric {
            key: "numbackends".to_string(),
            value: row.get::<_, i32>(0).to_string(),
            warning: false,
        });
        metrics.push(ServerMetric {
            key: "xact_commit".to_string(),
            value: row.get::<_, i64>(1).to_string(),
            warning: false,
        });
        metrics.push(ServerMetric {
            key: "blks_hit".to_string(),
            value: row.get::<_, i64>(2).to_string(),
            warning: false,
        });
        metrics.push(ServerMetric {
            key: "blks_read".to_string(),
            value: row.get::<_, i64>(3).to_string(),
            warning: false,
        });
        metrics.push(ServerMetric {
            key: "deadlocks".to_string(),
            value: row.get::<_, i64>(4).to_string(),
            warning: true,
        });
        metrics.push(ServerMetric {
            key: "temp_bytes".to_string(),
            value: row.get::<_, i64>(5).to_string(),
            warning: false,
        });
        metrics.push(ServerMetric {
            key: "tup_returned".to_string(),
            value: row.get::<_, i64>(6).to_string(),
            warning: false,
        });
        metrics.push(ServerMetric {
            key: "tup_fetched".to_string(),
            value: row.get::<_, i64>(7).to_string(),
            warning: false,
        });
    }
    if let Ok(row) = client
        .query_one(
            "SELECT count(*) FILTER (WHERE state = 'active' AND pid <> pg_backend_pid()), \
                    (SELECT count(*) FROM pg_locks WHERE NOT granted) \
             FROM pg_stat_activity",
            &[],
        )
        .await
    {
        metrics.push(ServerMetric {
            key: "active_queries".to_string(),
            value: row.get::<_, i64>(0).to_string(),
            warning: false,
        });
        metrics.push(ServerMetric {
            key: "lock_waits".to_string(),
            value: row.get::<_, i64>(1).to_string(),
            warning: true,
        });
    }
    if let Ok(row) = client.query_one("SHOW max_connections", &[]).await {
        metrics.push(ServerMetric {
            key: "max_connections".to_string(),
            value: row.get::<_, String>(0),
            warning: false,
        });
    }
    Ok(ServerStatus {
        uptime_secs,
        metrics,
    })
}

fn pg_kill(config: &ConnectionConfig, session_id: &str, query_only: bool) -> fluxdb_core::Result<()> {
    if config.is_read_only() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "只读连接不允许终止查询或会话",
        ));
    }
    let target = session_id
        .trim()
        .parse::<i32>()
        .map_err(|_| Error::new(ErrorKind::Query, "PostgreSQL 会话 ID 必须是数字 pid"))?;
    let database = pg_request_database(config, None);
    pg_runtime().block_on(async {
        let session = pg_connect(config, &database).await?;
        let client = session.client.as_ref();
        let own_pid = client
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .map_err(pg_error)?
            .get::<_, i32>(0);
        if own_pid == target {
            return Err(Error::new(
                ErrorKind::Query,
                "不能终止服务器面板自身使用的会话",
            ));
        }
        let sql = if query_only {
            "SELECT pg_cancel_backend($1)"
        } else {
            "SELECT pg_terminate_backend($1)"
        };
        let succeeded = client
            .query_one(sql, &[&target])
            .await
            .map_err(pg_error)?
            .get::<_, bool>(0);
        if !succeeded {
            return Err(Error::new(
                ErrorKind::Query,
                "未能终止目标会话：权限不足或会话已结束",
            ));
        }
        Ok(())
    })
}

fn pg_server_maintenance(
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
        pg_request_database(config, None)
    } else {
        request.database.trim().to_string()
    };
    let statement = match request.action {
        ServerMaintenanceAction::Vacuum => "VACUUM".to_string(),
        ServerMaintenanceAction::Analyze => "ANALYZE".to_string(),
        ServerMaintenanceAction::Reindex => format!(
            "REINDEX DATABASE {}",
            fluxdb_core::PostgresDialect.quote_identifier(&database)
        ),
        ServerMaintenanceAction::Optimize => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "该维护动作不适用于 PostgreSQL",
            ));
        }
    };
    pg_runtime().block_on(async {
        let session = pg_connect(config, &database).await?;
        session
            .client
            .batch_execute(&statement)
            .await
            .map_err(pg_error)?;
        Ok(String::new())
    })
}
