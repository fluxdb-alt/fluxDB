// Oracle 服务器活动：V$SESSION / V$SQL / V$LOCK 快照、锁等待与终止会话。
//
// 一次读取建一条短连接；锁与状态分区独立报错，避免 V$ 视图权限不足时整页不可用。
// 会话标识统一为 `sid,serial#`，终止命令与展示使用同一格式。

fn oracle_server_activity(config: &ConnectionConfig) -> fluxdb_core::Result<ServerActivity> {
    let connection = oracle_connect(config)?;
    let version = oracle_query(
        &connection,
        "SELECT version FROM v$instance",
        &[],
    )
    .ok()
    .and_then(|rows| rows.first().and_then(|row| oracle_meta_text(row, 0)))
    .unwrap_or_default();
    let current_user = oracle_query(&connection, "SELECT USER FROM dual", &[])
        .ok()
        .and_then(|rows| rows.first().and_then(|row| oracle_meta_text(row, 0)))
        .unwrap_or_default();
    let own_sid = oracle_query(
        &connection,
        "SELECT SYS_CONTEXT('USERENV', 'SID') FROM dual",
        &[],
    )
    .ok()
    .and_then(|rows| rows.first().and_then(|row| oracle_meta_text(row, 0)))
    .and_then(|sid| sid.trim().parse::<i64>().ok());

    let sessions = oracle_activity_sessions(&connection, own_sid)?;
    let (locks, locks_error) = match oracle_activity_locks(&connection) {
        Ok(locks) => (locks, None),
        Err(error) => (Vec::new(), Some(error.message)),
    };
    let (status, status_error) = match oracle_activity_status(&connection) {
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
}

fn oracle_activity_sessions(
    connection: &OracleConnection,
    own_sid: Option<i64>,
) -> fluxdb_core::Result<Vec<ServerSession>> {
    let rows = oracle_query(
        connection,
        "SELECT s.sid, s.serial#, s.username, s.machine, COALESCE(s.service_name, ''), \
                s.status, s.last_call_et, COALESCE(s.event, ''), COALESCE(q.sql_text, '') \
         FROM v$session s \
         LEFT JOIN v$sql q ON q.sql_id = s.sql_id AND q.child_number = s.sql_child_number \
         WHERE s.type = 'USER' AND s.username IS NOT NULL \
         ORDER BY s.last_call_et DESC",
        &[],
    )?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let sid = oracle_meta_i64(&row, 0).unwrap_or(0);
            let serial = oracle_meta_i64(&row, 1).unwrap_or(0);
            ServerSession {
                id: format!("{sid},{serial}"),
                user: oracle_meta_text(&row, 2).unwrap_or_default(),
                host: oracle_meta_text(&row, 3).unwrap_or_default(),
                database: oracle_meta_text(&row, 4).unwrap_or_default(),
                command: oracle_meta_text(&row, 5).unwrap_or_default(),
                elapsed_secs: oracle_meta_i64(&row, 6).unwrap_or(0).max(0) as u64,
                wait_event: oracle_meta_text(&row, 7).unwrap_or_default(),
                sql: oracle_meta_text(&row, 8).unwrap_or_default(),
                is_self: own_sid == Some(sid),
            }
        })
        .collect())
}

fn oracle_activity_locks(
    connection: &OracleConnection,
) -> fluxdb_core::Result<Vec<ServerLockWait>> {
    let rows = oracle_query(
        connection,
        "SELECT s.sid, s.serial#, s.blocking_session, bs.serial#, \
                COALESCE(s.event, ''), s.seconds_in_wait, COALESCE(q.sql_text, ''), \
                COALESCE(o.owner || '.' || o.object_name, ''), COALESCE(l.type, '') \
         FROM v$session s \
         JOIN v$session bs ON bs.sid = s.blocking_session \
         LEFT JOIN v$sql q ON q.sql_id = s.sql_id AND q.child_number = 0 \
         LEFT JOIN v$lock l ON l.sid = s.sid AND l.request > 0 \
         LEFT JOIN all_objects o ON o.object_id = s.row_wait_obj# \
         WHERE s.blocking_session IS NOT NULL \
         ORDER BY s.seconds_in_wait DESC",
        &[],
    )?;
    // 一个等待会话可能持有多个未授予锁，按「等待者/阻塞者/对象/锁类型」去重。
    let mut seen = std::collections::BTreeSet::new();
    let mut locks = Vec::new();
    for row in rows {
        let waiting_session = format!(
            "{},{}",
            oracle_meta_i64(&row, 0).unwrap_or(0),
            oracle_meta_i64(&row, 1).unwrap_or(0)
        );
        let blocking_session = format!(
            "{},{}",
            oracle_meta_i64(&row, 2).unwrap_or(0),
            oracle_meta_i64(&row, 3).unwrap_or(0)
        );
        let object = oracle_meta_text(&row, 7).unwrap_or_default();
        let lock_type = oracle_meta_text(&row, 8).unwrap_or_default();
        if !seen.insert((
            waiting_session.clone(),
            blocking_session.clone(),
            object.clone(),
            lock_type.clone(),
        )) {
            continue;
        }
        locks.push(ServerLockWait {
            waiting_session,
            blocking_session,
            object,
            lock_type,
            wait_secs: oracle_meta_i64(&row, 5).unwrap_or(0).max(0) as u64,
            state: oracle_meta_text(&row, 4).unwrap_or_default(),
            sql: oracle_meta_text(&row, 6).unwrap_or_default(),
        });
    }
    Ok(locks)
}

fn oracle_activity_status(
    connection: &OracleConnection,
) -> fluxdb_core::Result<ServerStatus> {
    let uptime_secs = oracle_query(
        connection,
        "SELECT ROUND((SYSDATE - startup_time) * 86400) FROM v$instance",
        &[],
    )
    .ok()
    .and_then(|rows| rows.first().and_then(|row| oracle_meta_i64(row, 0)))
    .map(|value| value.max(0) as u64);

    let rows = oracle_query(
        connection,
        "SELECT (SELECT COUNT(*) FROM v$session WHERE type = 'USER'), \
                (SELECT COUNT(*) FROM v$session WHERE type = 'USER' AND status = 'ACTIVE'), \
                (SELECT COUNT(*) FROM v$session WHERE blocking_session IS NOT NULL), \
                (SELECT value FROM v$sysstat WHERE name = 'session logical reads'), \
                (SELECT value FROM v$sysstat WHERE name = 'physical reads'), \
                (SELECT value FROM v$sysstat WHERE name = 'execute count') \
         FROM dual",
        &[],
    )?;
    let Some(row) = rows.first() else {
        return Ok(ServerStatus::default());
    };
    let metric = |index: usize, key: &str, warning: bool| ServerMetric {
        key: key.to_string(),
        value: oracle_meta_text(row, index).unwrap_or_default(),
        warning,
    };
    Ok(ServerStatus {
        uptime_secs,
        metrics: vec![
            metric(0, "sessions", false),
            metric(1, "active_sessions", false),
            metric(2, "lock_waits", true),
            metric(3, "session_logical_reads", false),
            metric(4, "physical_reads", false),
            metric(5, "execute_count", false),
        ],
    })
}

fn oracle_kill(config: &ConnectionConfig, session_id: &str, query_only: bool) -> fluxdb_core::Result<()> {
    if config.is_read_only() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "只读连接不允许终止查询或会话",
        ));
    }
    // oracledb 驱动收到 CANCEL 引起的 ORA-01013 后会卡在 Client::reset 等不到复位包，
    // 被取消的查询线程无法返回。服务端取消本身可用，但驱动侧不可用，先明确拒绝。
    if query_only {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "Oracle 暂不支持只取消查询，请终止会话",
        ));
    }
    let (sid, serial) = oracle_parse_session_id(session_id)?;
    let connection = oracle_connect(config)?;
    let own_sid = oracle_query(
        &connection,
        "SELECT SYS_CONTEXT('USERENV', 'SID') FROM dual",
        &[],
    )
    .ok()
    .and_then(|rows| rows.first().and_then(|row| oracle_meta_i64(row, 0)));
    if own_sid == Some(sid) {
        return Err(Error::new(
            ErrorKind::Query,
            "不能终止服务器面板自身使用的会话",
        ));
    }
    let sql = format!("ALTER SYSTEM KILL SESSION '{sid},{serial}' IMMEDIATE");
    match oracle_execute(&connection, &sql, &[]) {
        Ok(_) => Ok(()),
        // Oracle 在会话已进入终止流程时返回 ORA-00031；目标会话已被关闭，按成功处理。
        Err(error) if error.message.contains("ORA-00031") => Ok(()),
        Err(error) => Err(error),
    }
}

fn oracle_parse_session_id(session_id: &str) -> fluxdb_core::Result<(i64, i64)> {
    let (sid, serial) = session_id
        .trim()
        .split_once(',')
        .ok_or_else(|| Error::new(ErrorKind::Query, "Oracle 会话 ID 必须是 sid,serial#"))?;
    Ok((
        sid.trim()
            .parse()
            .map_err(|_| Error::new(ErrorKind::Query, "Oracle 会话 sid 不合法"))?,
        serial
            .trim()
            .parse()
            .map_err(|_| Error::new(ErrorKind::Query, "Oracle 会话 serial# 不合法"))?,
    ))
}

fn oracle_server_maintenance(
    config: &ConnectionConfig,
    request: &ServerMaintenanceRequest,
) -> fluxdb_core::Result<String> {
    if config.is_read_only() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "只读连接不允许执行维护",
        ));
    }
    let statement = match request.action {
        ServerMaintenanceAction::Analyze => {
            "BEGIN DBMS_STATS.GATHER_SCHEMA_STATS(ownname => USER); END;".to_string()
        }
        ServerMaintenanceAction::Reindex => {
            "BEGIN FOR r IN (SELECT index_name FROM user_indexes WHERE status <> 'UNUSABLE') \
             LOOP EXECUTE IMMEDIATE 'ALTER INDEX \"' || REPLACE(r.index_name, '\"', '\"\"') || \
             '\" REBUILD'; END LOOP; END;"
                .to_string()
        }
        ServerMaintenanceAction::Vacuum | ServerMaintenanceAction::Optimize => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "该维护动作不适用于 Oracle",
            ));
        }
    };
    let connection = oracle_connect(config)?;
    oracle_execute(&connection, &statement, &[]).map(|_| String::new())
}
