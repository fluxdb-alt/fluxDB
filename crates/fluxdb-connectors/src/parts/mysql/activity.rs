// MySQL 服务器活动：会话、锁等待、实例状态与 KILL。
//
// 复用 `mysql_connection_plan`（SSH/代理/TLS/超时），每次读取建立一条短连接。
// 锁等待与状态分区各自吞掉可预期错误：权限不足时返回明确原因，而不是让整页失败。

fn mysql_server_activity(config: &ConnectionConfig) -> fluxdb_core::Result<ServerActivity> {
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = plan.connect().await?;
        let version = mysql_query_scalar_string(&mut connection, "SELECT VERSION()")
            .await
            .unwrap_or_default();
        let current_user = mysql_query_scalar_string(&mut connection, "SELECT CURRENT_USER()")
            .await
            .unwrap_or_default();
        let own_id = sqlx::query_scalar::<_, u64>("SELECT CONNECTION_ID()")
            .fetch_one(&mut connection)
            .await
            .ok();

        let sessions = mysql_read_sessions(&mut connection, own_id).await?;
        let (locks, locks_error) = match mysql_read_lock_waits(&mut connection).await {
            Ok(locks) => (locks, None),
            Err(error) => (Vec::new(), Some(error.message)),
        };
        let (status, status_error) = match mysql_read_status(&mut connection).await {
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

async fn mysql_query_scalar_string(
    connection: &mut MySqlConnection,
    sql: &str,
) -> fluxdb_core::Result<String> {
    sqlx::query_scalar::<_, String>(sql)
        .fetch_one(&mut *connection)
        .await
        .map_err(mysql_error)
}

async fn mysql_read_sessions(
    connection: &mut MySqlConnection,
    own_id: Option<u64>,
) -> fluxdb_core::Result<Vec<ServerSession>> {
    // PROCESS 权限决定能看到多少会话；没有权限时服务端只返回当前账号自己的会话。
    let rows = sqlx::query(
        "SELECT ID, `USER` AS user_name, HOST, DB, COMMAND, TIME, STATE, INFO \
         FROM information_schema.PROCESSLIST \
         WHERE COMMAND <> 'Daemon' \
         ORDER BY TIME DESC \
         LIMIT 500",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(mysql_error)?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let id = mysql_row_u64(&row, "ID");
            ServerSession {
                id: id.to_string(),
                is_self: own_id == Some(id),
                user: mysql_row_string(&row, "user_name"),
                host: mysql_row_string(&row, "HOST"),
                database: mysql_row_string(&row, "DB"),
                command: mysql_row_string(&row, "COMMAND"),
                elapsed_secs: mysql_row_u64(&row, "TIME"),
                wait_event: mysql_row_string(&row, "STATE"),
                sql: mysql_row_string(&row, "INFO"),
            }
        })
        .collect())
}

/// 读取锁等待。优先 MySQL 8.0 `performance_schema.data_lock_waits`；失败时回退
/// 5.7/MariaDB 的 `information_schema.innodb_lock_waits`。
async fn mysql_read_lock_waits(
    connection: &mut MySqlConnection,
) -> fluxdb_core::Result<Vec<ServerLockWait>> {
    let rows = sqlx::query(
        "SELECT w.REQUESTING_THREAD_ID AS waiting_thread, \
                w.BLOCKING_THREAD_ID AS blocking_thread, \
                COALESCE(dl.OBJECT_SCHEMA, '') AS object_schema, \
                COALESCE(dl.OBJECT_NAME, '') AS object_name, \
                COALESCE(dl.LOCK_TYPE, '') AS lock_type, \
                COALESCE(dl.LOCK_MODE, '') AS lock_mode, \
                COALESCE(TIMESTAMPDIFF(SECOND, trx.trx_started, NOW()), 0) AS wait_secs, \
                COALESCE(p.STATE, '') AS state, \
                COALESCE(p.INFO, '') AS sql_text \
         FROM performance_schema.data_lock_waits w \
         LEFT JOIN performance_schema.data_locks dl \
           ON dl.ENGINE = w.ENGINE AND dl.ENGINE_LOCK_ID = w.REQUESTING_ENGINE_LOCK_ID \
         LEFT JOIN information_schema.innodb_trx trx \
           ON trx.trx_mysql_thread_id = w.REQUESTING_THREAD_ID \
         LEFT JOIN information_schema.PROCESSLIST p ON p.ID = w.REQUESTING_THREAD_ID \
         ORDER BY wait_secs DESC",
    )
    .fetch_all(&mut *connection)
    .await;

    match rows {
        Ok(rows) => Ok(rows
            .into_iter()
            .map(|row| {
                let schema = mysql_row_string(&row, "object_schema");
                let name = mysql_row_string(&row, "object_name");
                ServerLockWait {
                    waiting_session: mysql_row_u64(&row, "waiting_thread").to_string(),
                    blocking_session: mysql_row_u64(&row, "blocking_thread").to_string(),
                    object: if schema.is_empty() {
                        name
                    } else {
                        format!("{schema}.{name}")
                    },
                    lock_type: mysql_row_string(&row, "lock_type"),
                    wait_secs: mysql_row_u64(&row, "wait_secs"),
                    state: "waiting".to_string(),
                    sql: mysql_row_string(&row, "sql_text"),
                }
            })
            .collect()),
        Err(primary_error) => {
            // 旧版本 / MariaDB：只有 InnoDB 事务级等待边，拿不到具体对象。
            let fallback = sqlx::query(
                "SELECT r.trx_mysql_thread_id AS waiting_thread, \
                        b.trx_mysql_thread_id AS blocking_thread, \
                        COALESCE(TIMESTAMPDIFF(SECOND, r.trx_wait_started, NOW()), 0) AS wait_secs, \
                        COALESCE(r.trx_query, '') AS sql_text \
                 FROM information_schema.innodb_lock_waits w \
                 JOIN information_schema.innodb_trx r ON r.trx_id = w.requesting_trx_id \
                 JOIN information_schema.innodb_trx b ON b.trx_id = w.blocking_trx_id \
                 ORDER BY wait_secs DESC",
            )
            .fetch_all(&mut *connection)
            .await;
            match fallback {
                Ok(rows) => Ok(rows
                    .into_iter()
                    .map(|row| ServerLockWait {
                        waiting_session: mysql_row_u64(&row, "waiting_thread").to_string(),
                        blocking_session: mysql_row_u64(&row, "blocking_thread").to_string(),
                        object: String::new(),
                        lock_type: "row".to_string(),
                        wait_secs: mysql_row_u64(&row, "wait_secs"),
                        state: "waiting".to_string(),
                        sql: mysql_row_string(&row, "sql_text"),
                    })
                    .collect()),
                Err(_) => Err(mysql_error(primary_error)),
            }
        }
    }
}

async fn mysql_read_status(
    connection: &mut MySqlConnection,
) -> fluxdb_core::Result<ServerStatus> {
    let rows = sqlx::query("SHOW GLOBAL STATUS")
        .fetch_all(&mut *connection)
        .await
        .map_err(mysql_error)?;
    let mut values = std::collections::BTreeMap::<String, String>::new();
    for row in rows {
        values.insert(
            mysql_row_string(&row, "Variable_name"),
            mysql_row_string(&row, "Value"),
        );
    }

    let max_connections = sqlx::query_scalar::<_, String>(
        "SELECT VARIABLE_VALUE FROM performance_schema.global_variables \
         WHERE VARIABLE_NAME = 'max_connections'",
    )
    .fetch_one(&mut *connection)
    .await
    .ok();
    let mut metrics = Vec::new();
    let mut push = |key: &str, warning: bool| {
        if let Some(value) = values.get(key) {
            metrics.push(ServerMetric {
                key: key.to_string(),
                value: value.clone(),
                warning,
            });
        }
    };
    push("Threads_connected", false);
    push("Threads_running", false);
    push("Innodb_row_lock_waits", true);
    push("Slow_queries", true);
    push("Queries", false);
    push("Max_used_connections", false);
    if let Some(max_connections) = max_connections {
        metrics.push(ServerMetric {
            key: "max_connections".to_string(),
            value: max_connections,
            warning: false,
        });
    }
    let uptime_secs = values.get("Uptime").and_then(|value| value.parse().ok());
    Ok(ServerStatus {
        uptime_secs,
        metrics,
    })
}

fn mysql_kill(config: &ConnectionConfig, session_id: &str, query_only: bool) -> fluxdb_core::Result<()> {
    if config.is_read_only() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "只读连接不允许终止查询或会话",
        ));
    }
    let target = session_id
        .trim()
        .parse::<u64>()
        .map_err(|_| Error::new(ErrorKind::Query, "MySQL 会话 ID 必须是数字"))?;
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;
    runtime.block_on(async {
        let mut connection = plan.connect().await?;
        let own_id = sqlx::query_scalar::<_, u64>("SELECT CONNECTION_ID()")
            .fetch_one(&mut connection)
            .await
            .map_err(mysql_error)?;
        if own_id == target {
            return Err(Error::new(
                ErrorKind::Query,
                "不能终止服务器面板自身使用的会话",
            ));
        }
        let statement = if query_only {
            format!("KILL QUERY {target}")
        } else {
            format!("KILL CONNECTION {target}")
        };
        sqlx::raw_sql(&statement)
            .execute(&mut connection)
            .await
            .map_err(mysql_error)?;
        Ok(())
    })
}

fn mysql_server_maintenance(
    config: &ConnectionConfig,
    request: &ServerMaintenanceRequest,
) -> fluxdb_core::Result<String> {
    if config.is_read_only() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "只读连接不允许执行维护",
        ));
    }
    let statement_prefix = match request.action {
        ServerMaintenanceAction::Analyze => "ANALYZE TABLE",
        ServerMaintenanceAction::Optimize => "OPTIMIZE TABLE",
        ServerMaintenanceAction::Vacuum | ServerMaintenanceAction::Reindex => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "该维护动作不适用于 MySQL",
            ));
        }
    };
    let database = request.database.trim();
    if database.is_empty() {
        return Err(Error::new(ErrorKind::Query, "维护需要指定数据库"));
    }
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;
    runtime.block_on(async {
        let mut connection = plan.connect().await?;
        let tables = sqlx::query_scalar::<_, String>(
            "SELECT TABLE_NAME FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = ? AND TABLE_TYPE = 'BASE TABLE' \
             ORDER BY TABLE_NAME",
        )
        .bind(database)
        .fetch_all(&mut connection)
        .await
        .map_err(mysql_error)?;
        if tables.is_empty() {
            return Ok(String::new());
        }
        let quoted_database = fluxdb_core::MySqlDialect.quote_identifier(database);
        // 每批 50 张表，避免单条语句过长；逐批执行，任一批失败即中止并报错。
        for chunk in tables.chunks(50) {
            let names = chunk
                .iter()
                .map(|table| {
                    format!(
                        "{quoted_database}.{}",
                        fluxdb_core::MySqlDialect.quote_identifier(table)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            sqlx::raw_sql(&format!("{statement_prefix} {names}"))
                .fetch_all(&mut connection)
                .await
                .map_err(mysql_error)?;
        }
        Ok(String::new())
    })
}

fn mysql_row_string(row: &MySqlRow, column: &str) -> String {
    row.try_get::<Option<String>, _>(column)
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn mysql_row_u64(row: &MySqlRow, column: &str) -> u64 {
    row.try_get::<u64, _>(column)
        .or_else(|_| {
            row.try_get::<i64, _>(column)
                .map(|value| value.max(0) as u64)
        })
        .unwrap_or(0)
}
