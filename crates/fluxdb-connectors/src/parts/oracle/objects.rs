// Oracle 对象树：连接 → 容器（库/PDB）→ Owner（schema）→ 表/视图。
//
// 只用 `ALL_*` 与 `USERENV` 上下文，不要求 `DBA_*` / `V$*` 权限；看不到的对象
// 就不出现，不伪造空列表来掩盖权限不足。

fn oracle_list_objects(
    config: &ConnectionConfig,
    path: Option<&ObjectPath>,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    if config.kind != DatabaseKind::Oracle {
        return Err(Error::new(ErrorKind::Connection, "连接类型不匹配"));
    }
    match path {
        None => oracle_list_databases(config),
        Some(path) if path.kind == ObjectKind::Database => oracle_list_schemas(config, path),
        Some(path) if path.kind == ObjectKind::Schema => oracle_list_relations(config, path),
        Some(_) => Err(Error::new(ErrorKind::Internal, "不支持的 Oracle 对象层级")),
    }
}

/// 根节点是当前连接所在容器。CDB 下 `CON_NAME` 是 PDB 名，非 CDB 时回落到库名。
fn oracle_list_databases(config: &ConnectionConfig) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let connection = oracle_connect(config)?;
    let rows = oracle_query(
        &connection,
        "SELECT SYS_CONTEXT('USERENV','CON_NAME') AS CON_NAME, \
                SYS_CONTEXT('USERENV','DB_NAME') AS DB_NAME FROM DUAL",
        &[],
    )?;
    let name = rows
        .first()
        .and_then(|row| oracle_meta_text(row, 0))
        .or_else(|| rows.first().and_then(|row| oracle_meta_text(row, 1)))
        .ok_or_else(|| Error::new(ErrorKind::Query, "未读取到 Oracle 容器名"))?;
    Ok(vec![ObjectSummary {
        path: ObjectPath {
            connection_id: config.id,
            database: Some(name.clone()),
            schema: None,
            name,
            kind: ObjectKind::Database,
        },
        rows: None,
        size_bytes: None,
        modified_at: None,
        comment: None,
        stable: None,
    }])
}

/// Owner 即 schema。`ALL_USERS` 只返回当前会话可见的账号，不做权限探测。
fn oracle_list_schemas(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let database = path
        .database
        .clone()
        .filter(|database| !database.trim().is_empty())
        .unwrap_or_else(|| path.name.clone());
    let connection = oracle_connect(config)?;
    let rows = oracle_query(
        &connection,
        "SELECT USERNAME FROM ALL_USERS WHERE ORACLE_MAINTAINED = 'N' ORDER BY USERNAME",
        &[],
    )?;
    Ok(rows
        .iter()
        .filter_map(|row| oracle_meta_text(row, 0))
        .map(|name| ObjectSummary {
            path: ObjectPath {
                connection_id: config.id,
                database: Some(database.clone()),
                schema: None,
                name,
                kind: ObjectKind::Schema,
            },
            rows: None,
            size_bytes: None,
            modified_at: None,
            comment: None,
            stable: None,
        })
        .collect())
}

fn oracle_list_relations(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let database = path.database.clone();
    let requested = path.name.clone();
    let connection = oracle_connect(config)?;
    let owner = oracle_resolve_owner(&connection, &requested)?;
    // 只列可浏览的表与视图：物化视图按视图展示；同义词/序列/过程不在数据页范畴，
    // 需要时另行接入，避免在这里给出点开却没有数据的节点。
    //
    // 行数取 `ALL_TABLES.NUM_ROWS`（优化器统计值，与 MySQL 的 `information_schema.tables`
    // 同一性质）：它可能滞后于真实行数、也可能因为从未收集统计而为 NULL，此时按「未知」
    // 留空，不用 `COUNT(*)` 去把树加载变成全表扫描。
    let rows = oracle_query(
        &connection,
        "SELECT O.OBJECT_NAME, O.OBJECT_TYPE, O.OBJECT_ID, \
                TO_CHAR(O.LAST_DDL_TIME, 'YYYY-MM-DD HH24:MI:SS') AS MODIFIED_AT, \
                C.COMMENTS, T.NUM_ROWS \
         FROM ALL_OBJECTS O \
         LEFT JOIN ALL_TAB_COMMENTS C \
           ON C.OWNER = O.OWNER AND C.TABLE_NAME = O.OBJECT_NAME AND C.TABLE_TYPE = O.OBJECT_TYPE \
         LEFT JOIN ALL_TABLES T \
           ON T.OWNER = O.OWNER AND T.TABLE_NAME = O.OBJECT_NAME \
         WHERE O.OWNER = :1 AND O.OBJECT_TYPE IN ('TABLE','VIEW','MATERIALIZED VIEW') \
           AND O.GENERATED = 'N' AND O.SECONDARY = 'N' \
         ORDER BY O.OBJECT_NAME",
        &[OracleBindValue::Text(owner.clone())],
    )?;
    let mut objects = Vec::with_capacity(rows.len());
    for row in &rows {
        let Some(name) = oracle_meta_text(row, 0) else {
            continue;
        };
        let object_type = oracle_meta_text(row, 1).unwrap_or_default();
        objects.push(ObjectSummary {
            path: ObjectPath {
                connection_id: config.id,
                database: database.clone(),
                schema: Some(owner.clone()),
                name,
                kind: if object_type.eq_ignore_ascii_case("TABLE") {
                    ObjectKind::Table
                } else {
                    ObjectKind::View
                },
            },
            rows: oracle_meta_u64(row, 5),
            size_bytes: None,
            modified_at: oracle_meta_text(row, 3),
            comment: oracle_meta_text(row, 4),
            stable: oracle_meta_u64(row, 2),
        });
    }
    Ok(objects)
}
