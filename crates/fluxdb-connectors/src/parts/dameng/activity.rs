// 达梦服务器活动：V$SESSIONS / V$TRXWAIT / V$LOCK 快照与终止会话。
//
// 一次读取建一条短连接；锁与状态分区独立报错。会话 ID 直接使用 SESS_ID，
// 查询取消与会话关闭分别调用 SP_CANCEL_SESSION_OPERATION / SP_CLOSE_SESSION。

fn dm_server_activity(config: &ConnectionConfig) -> fluxdb_core::Result<ServerActivity> {
    let mut client = dm_connect(config)?;
    let version = dm_activity_query(&mut client, "SELECT BUILD_VERSION FROM V$INSTANCE", &[])
        .ok()
        .and_then(|rows| rows.first().and_then(|row| dm_cell(row, 0)))
        .unwrap_or_default();
    let database = dm_activity_query(&mut client, "SELECT NAME FROM V$DATABASE", &[])
        .ok()
        .and_then(|rows| rows.first().and_then(|row| dm_cell(row, 0)))
        .unwrap_or_default();
    let current_user = dm_activity_query(&mut client, "SELECT USER FROM DUAL", &[])
        .ok()
        .and_then(|rows| rows.first().and_then(|row| dm_cell(row, 0)))
        .unwrap_or_default();
    let own_sid = dm_activity_query(&mut client, "SELECT SESSID()", &[])
        .ok()
        .and_then(|rows| rows.first().and_then(|row| dm_value_i64(&row[0])));

    let sessions = dm_activity_sessions(&mut client, own_sid, &database)?;
    let (locks, locks_error) = match dm_activity_locks(&mut client) {
        Ok(locks) => (locks, None),
        Err(error) => (Vec::new(), Some(error.message)),
    };
    let (status, status_error) = match dm_activity_status(&mut client) {
        Ok(status) => (status, None),
        Err(error) => (ServerStatus::default(), Some(error.message)),
    };
    client.close();
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

fn dm_activity_query(
    client: &mut DmClient,
    sql: &str,
    params: &[Param],
) -> fluxdb_core::Result<Vec<Vec<Value>>> {
    match dm_execute(client, sql, params)? {
        ExecuteResult::Query { rows, .. } => Ok(rows),
        _ => Err(Error::new(ErrorKind::Query, "达梦服务器状态查询返回了非结果集")),
    }
}

fn dm_activity_sessions(
    client: &mut DmClient,
    own_sid: Option<i64>,
    database: &str,
) -> fluxdb_core::Result<Vec<ServerSession>> {
    let rows = dm_activity_query(
        client,
        "SELECT SESS_ID, NVL(USER_NAME, ''), NVL(CLNT_IP, ''), NVL(STATE, ''), \
                DATEDIFF(SS, CREATE_TIME, SYSDATE), NVL(SQL_TEXT, '') \
         FROM V$SESSIONS \
         WHERE USER_NAME IS NOT NULL \
         ORDER BY 5 DESC",
        &[],
    )?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let id = row.first().and_then(dm_value_i64).unwrap_or(0);
            ServerSession {
                id: id.to_string(),
                user: dm_cell(&row, 1).unwrap_or_default(),
                host: dm_cell(&row, 2).unwrap_or_default(),
                database: database.to_string(),
                command: dm_cell(&row, 3).unwrap_or_default(),
                elapsed_secs: row
                    .get(4)
                    .and_then(dm_value_i64)
                    .unwrap_or(0)
                    .max(0) as u64,
                wait_event: String::new(),
                sql: dm_cell(&row, 5).unwrap_or_default(),
                is_self: own_sid == Some(id),
            }
        })
        .collect())
}

fn dm_activity_locks(client: &mut DmClient) -> fluxdb_core::Result<Vec<ServerLockWait>> {
    let rows = dm_activity_query(
        client,
        "SELECT WS.SESS_ID, BS.SESS_ID, NVL(WS.SQL_TEXT, ''), NVL(O.NAME, ''), \
                NVL(L.LTYPE, ''), NVL(W.WAIT_TIME / 1000, 0) \
         FROM V$TRXWAIT W \
         JOIN V$SESSIONS WS ON WS.TRX_ID = W.ID \
         JOIN V$SESSIONS BS ON BS.TRX_ID = W.WAIT_FOR_ID \
         LEFT JOIN V$LOCK L ON L.TRX_ID = W.ID AND L.BLOCKED = 1 \
         LEFT JOIN SYS.SYSOBJECTS O ON O.ID = L.TABLE_ID \
         ORDER BY 6 DESC",
        &[],
    )?;
    // 一个等待事务可能对应多把锁，按「等待者/阻塞者/对象/锁类型」去重。
    let mut seen = std::collections::BTreeSet::new();
    let mut locks = Vec::new();
    for row in rows {
        let waiting = row.first().and_then(dm_value_i64).unwrap_or(0);
        let blocking = row.get(1).and_then(dm_value_i64).unwrap_or(0);
        let object = dm_cell(&row, 3).unwrap_or_default();
        let lock_type = dm_cell(&row, 4).unwrap_or_default();
        if !seen.insert((waiting, blocking, object.clone(), lock_type.clone())) {
            continue;
        }
        locks.push(ServerLockWait {
            waiting_session: waiting.to_string(),
            blocking_session: blocking.to_string(),
            object,
            lock_type,
            wait_secs: row.get(5).and_then(dm_value_i64).unwrap_or(0).max(0) as u64,
            state: "waiting".to_string(),
            sql: dm_cell(&row, 2).unwrap_or_default(),
        });
    }
    Ok(locks)
}

fn dm_activity_status(client: &mut DmClient) -> fluxdb_core::Result<ServerStatus> {
    let rows = dm_activity_query(
        client,
        "SELECT (SELECT COUNT(*) FROM V$SESSIONS), \
                (SELECT COUNT(*) FROM V$SESSIONS WHERE STATE = 'ACTIVE'), \
                (SELECT COUNT(*) FROM V$TRXWAIT) \
         FROM DUAL",
        &[],
    )?;
    let Some(row) = rows.first() else {
        return Ok(ServerStatus::default());
    };
    Ok(ServerStatus {
        uptime_secs: None,
        metrics: vec![
            ServerMetric {
                key: "sessions".into(),
                value: dm_cell(row, 0).unwrap_or_default(),
                warning: false,
            },
            ServerMetric {
                key: "active_sessions".into(),
                value: dm_cell(row, 1).unwrap_or_default(),
                warning: false,
            },
            ServerMetric {
                key: "lock_waits".into(),
                value: dm_cell(row, 2).unwrap_or_default(),
                warning: true,
            },
        ],
    })
}

fn dm_kill(config: &ConnectionConfig, session_id: &str, query_only: bool) -> fluxdb_core::Result<()> {
    if config.is_read_only() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "只读连接不允许终止查询或会话",
        ));
    }
    let target = session_id
        .trim()
        .parse::<i64>()
        .map_err(|_| Error::new(ErrorKind::Query, "达梦会话 ID 必须是数字 SESS_ID"))?;
    let mut client = dm_connect(config)?;
    let own_sid = dm_activity_query(&mut client, "SELECT SESSID()", &[])
        .ok()
        .and_then(|rows| rows.first().and_then(|row| dm_value_i64(&row[0])));
    if own_sid == Some(target) {
        client.close();
        return Err(Error::new(
            ErrorKind::Query,
            "不能终止服务器面板自身使用的会话",
        ));
    }
    // 只接受已解析的数字，命令本身不拼接任何用户输入。
    let statement = if query_only {
        format!("SP_CANCEL_SESSION_OPERATION({target})")
    } else {
        format!("SP_CLOSE_SESSION({target})")
    };
    let result = dm_execute(&mut client, &statement, &[]);
    client.close();
    result.map(|_| ())
}
