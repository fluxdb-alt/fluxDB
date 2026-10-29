fn validate_data_changes(changes: &DataChangeSet) -> fluxdb_core::Result<()> {
    if changes.object.kind != ObjectKind::Table {
        return Err(Error::new(ErrorKind::Unsupported, "仅表对象支持数据提交"));
    }

    if changes.is_empty() {
        return Err(Error::new(ErrorKind::Query, "没有需要提交的更改"));
    }

    reject_truncated_values(changes)?;
    reject_truncated_identities(changes)
}

/// 拒绝用只加载了预览的值定位原始行。
///
/// 无主键表的行身份是整行值；其中若有大字段，身份里只有预览。按预览拼出的
/// WHERE 永远匹配不到库里的完整值，UPDATE/DELETE 会「成功」但命中 0 行：
/// 界面显示提交成功，刷新后行仍是原样或根本没删掉。此时无法安全定位该行，
/// 只能要求用户提供主键/唯一键。
fn reject_truncated_identities(changes: &DataChangeSet) -> fluxdb_core::Result<()> {
    let truncated = |identity: &fluxdb_core::RowIdentity| {
        identity.values.values().any(CellValue::is_truncated)
    };
    let found = changes
        .updates
        .iter()
        .any(|update| truncated(&update.identity))
        || changes.deletes.iter().any(truncated);
    if found {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "行身份包含只加载了预览的大字段，无法可靠定位原始行；请为该表添加主键或唯一键后重试",
        ));
    }
    Ok(())
}

/// 拒绝提交任何只加载了预览的值。
///
/// 预览是片段，写回去等于用片段覆盖完整数据，且不可逆。所有连接器的写入口都先经过
/// [`validate_data_changes`]，因此这一处校验覆盖 MySQL / SQLite / PostgreSQL / 达梦 /
/// SQL Server / Oracle 的全部写入路径。完整值应由上层先按需加载（详情面板）再提交。
fn reject_truncated_values(changes: &DataChangeSet) -> fluxdb_core::Result<()> {
    let truncated_cell = |value: &CellValue| value.is_truncated();
    let in_insert = changes
        .inserts
        .iter()
        .any(|row| row.values.iter().any(truncated_cell));
    let in_update = changes.updates.iter().any(|update| {
        update.cells.iter().any(|cell| {
            // 流式写入的占位值本身就是 `LargeValue`（只有内容引用与大小），
            // 正文在内容存储里，由连接器按块读取——这不是「只加载了预览」。
            cell.value.stream().is_none() && truncated_cell(cell.value.placeholder())
        })
    });
    if in_insert || in_update {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "提交包含只加载了预览的文本（截断的 CLOB），请先加载完整值再保存",
        ));
    }
    Ok(())
}

/// 拆出一行更新里的「普通值」与「流式值」，供连接器分别处理。
///
/// 同一行既有普通改动又有流式改动是允许的：普通列照常 SET，流式列按块追加。
fn split_update_cells(
    update: &RowUpdate,
) -> (
    Vec<&fluxdb_core::CellUpdate>,
    Vec<(&fluxdb_core::CellUpdate, &fluxdb_core::StreamedValue)>,
) {
    let mut plain = Vec::new();
    let mut streamed = Vec::new();
    for cell in &update.cells {
        match cell.value.stream() {
            Some(stream) => streamed.push((cell, stream)),
            None => plain.push(cell),
        }
    }
    (plain, streamed)
}

/// 读取流式内容的下一块（按 [`fluxdb_core::BLOB_STREAM_CHUNK_BYTES`] 步长）。
///
/// 返回 `None` 表示已经读完。连接器自己驱动「读一块、发一条」的循环：各库的执行
/// 都是异步的，闭包式回调没法在里面 `await`。进程内存峰值等于一块。
///
/// 读到空块但偏移还没走完，说明暂存内容被提前释放了（例如暂存归属被清理）：
/// 这时必须报错而不是当读完，否则会静默写入半份内容。
fn streamed_chunk_at(
    stream: &fluxdb_core::StreamedValue,
    offset: u64,
) -> fluxdb_core::Result<Option<Vec<u8>>> {
    if offset >= stream.byte_length {
        return Ok(None);
    }
    let store = fluxdb_core::cell_content_store().ok_or_else(|| {
        Error::new(
            ErrorKind::Internal,
            "大字段内容存储未初始化，无法读写完整内容",
        )
    })?;
    let take = (stream.byte_length - offset).min(fluxdb_core::BLOB_STREAM_CHUNK_BYTES as u64) as usize;
    let bytes = store.read(stream.content_id(), offset, take)?;
    if bytes.is_empty() {
        return Err(Error::new(
            ErrorKind::Internal,
            format!(
                "待写入内容提前结束（已读 {offset} / {} 字节），已取消提交",
                stream.byte_length
            ),
        ));
    }
    let bytes = if stream.is_text() {
        truncate_to_char_boundary(bytes, offset, stream.byte_length)?
    } else {
        bytes
    };
    Ok(Some(bytes))
}

/// 文本分块必须落在 UTF-8 字符边界上。
///
/// 为什么不能靠「服务端自己拼」：分块追加是多次独立写入，中途每一块都会被当作
/// 文本存进去；把多字节字符劈成两半会写进非法字节序列，整列文本从此不再合法。
/// 这里把块尾回退到最近的字符边界，剩下的字节留给下一块——**只回退不丢弃**。
///
/// 极端情况：单块里一个完整字符都装不下（分块阈值被调到几个字节），说明配置本身
/// 有问题，直接报错而不是死循环。
fn truncate_to_char_boundary(
    mut bytes: Vec<u8>,
    offset: u64,
    total: u64,
) -> fluxdb_core::Result<Vec<u8>> {
    let mut keep = bytes.len();
    while keep > 0 && std::str::from_utf8(&bytes[..keep]).is_err() {
        // 只在「结尾被截断」时回退；中间的非法字节属于内容本身有问题，
        // 继续回退永远拼不出合法文本，交给数据库如实报错。
        let consumed = offset + keep as u64;
        if consumed >= total {
            break;
        }
        let error = std::str::from_utf8(&bytes[..keep]).expect_err("just checked");
        if error.error_len().is_some() {
            break;
        }
        keep -= 1;
    }
    if keep == 0 && !bytes.is_empty() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "文本分块过小，无法切在字符边界上；请调大分块阈值后重试",
        ));
    }
    bytes.truncate(keep);
    Ok(bytes)
}

/// 流式写入开始前清空目标列的影响行数校验：必须恰好 1 行，否则取消。
///
/// 0 行=目标行在提交期间被删除；>1=身份条件不唯一。两种情况继续追加都会写错行。
fn require_single_streamed_row(affected: u64, column: &str) -> fluxdb_core::Result<()> {
    if affected == 1 {
        Ok(())
    } else {
        Err(streamed_row_lost_error(column))
    }
}

fn streamed_row_lost_error(column: &str) -> Error {
    Error::new(
        ErrorKind::Query,
        format!("字段 {column} 的流式写入没有命中唯一一行，已取消提交；请刷新后重试"),
    )
}

/// 尚未实现分块追加的连接器（Oracle / 达梦 / Redis / Mock）在入口显式拒绝流式值。
///
/// 必须显式拒绝：这些连接器的绑定函数只看占位值，若放过去就等于把「类型 + 大小」
/// 写进数据库，静默毁掉用户数据。
fn reject_streamed_values(changes: &DataChangeSet) -> fluxdb_core::Result<()> {
    if changes
        .updates
        .iter()
        .flat_map(|update| update.cells.iter())
        .any(|cell| cell.value.stream().is_some())
    {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "当前数据库暂不支持流式写入大字段，请改用较小的文件",
        ));
    }
    Ok(())
}

fn validate_identity(identity: &fluxdb_core::RowIdentity, columns: &[Column]) -> fluxdb_core::Result<()> {
    if identity.values.is_empty() {
        return Err(Error::new(ErrorKind::Query, "缺少行身份条件，已取消提交"));
    }

    for (column, value) in &identity.values {
        ensure_column_exists(column, columns)?;
        if value.is_truncated() || matches!(value, CellValue::BinarySummary(_)) {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "行身份包含未完整加载的大字段，无法安全定位行；请刷新后重试或改用主键定位",
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod identity_value_guard_tests {
    use super::validate_identity;
    use fluxdb_core::{CellValue, Column, RowIdentity};
    use std::collections::BTreeMap;

    fn columns() -> Vec<Column> {
        vec![
            Column {
                name: "id".to_string(),
                type_name: Some("text".to_string()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            Column {
                name: "payload".to_string(),
                type_name: Some("text".to_string()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
        ]
    }

    #[test]
    fn complete_identity_is_allowed() {
        let identity = RowIdentity {
            values: BTreeMap::from([("id".to_string(), CellValue::Text("id-1".to_string()))]),
        };
        validate_identity(&identity, &columns()).expect("完整身份可提交");
    }

    #[test]
    fn truncated_identity_is_rejected_for_shared_connectors() {
        for value in [
            CellValue::TruncatedText {
                preview: "prefix".to_string(),
                total_chars: 1000,
            },
            CellValue::LargeValue(fluxdb_core::LargeCellValue {
                content_id: "content-1".to_string(),
                kind: fluxdb_core::LargeValueKind::Text,
                byte_length: Some(1000),
                preview: "prefix".to_string(),
                preview_truncated: true,
            }),
            CellValue::BinarySummary(fluxdb_core::BinaryCellSummary {
                type_name: "bytea".to_string(),
                is_null: false,
                byte_length: 1000,
                preview_hex: Some("00ff".to_string()),
            }),
        ] {
            let identity = RowIdentity {
                values: BTreeMap::from([("payload".to_string(), value)]),
            };
            let error =
                validate_identity(&identity, &columns()).expect_err("预览身份必须被拒绝");
            assert_eq!(error.kind, fluxdb_core::ErrorKind::Unsupported);
        }
    }
}

fn ensure_column_exists(column: &str, columns: &[Column]) -> fluxdb_core::Result<()> {
    if columns.iter().any(|existing| existing.name == column) {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::Query,
            format!("字段不存在或已变更：{column}"),
        ))
    }
}

fn non_null_insert_values<'a>(
    row: &'a Row,
    columns: &'a [Column],
) -> fluxdb_core::Result<Vec<(&'a Column, &'a CellValue)>> {
    if row.values.len() != columns.len() {
        return Err(Error::new(
            ErrorKind::Query,
            "新增行字段数量和当前表结构不一致",
        ));
    }

    Ok(columns
        .iter()
        .zip(row.values.iter())
        .filter(|(_, value)| !matches!(value, CellValue::Null))
        .collect())
}

fn push_insert_column_list<DB>(
    builder: &mut QueryBuilder<'_, DB>,
    values: &[(&Column, &CellValue)],
    quote_identifier: fn(&str) -> String,
) where
    DB: sqlx::Database,
{
    for (index, (column, _)) in values.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push(quote_identifier(&column.name));
    }
}

fn push_mysql_bind(builder: &mut QueryBuilder<'_, MySql>, value: &CellValue) {
    match value {
        CellValue::Null => {
            builder.push_bind(Option::<String>::None);
        }
        CellValue::Bool(value) => {
            builder.push_bind(*value);
        }
        CellValue::I64(value) => {
            builder.push_bind(*value);
        }
        CellValue::F64(value) => {
            builder.push_bind(*value);
        }
        CellValue::Text(value) | CellValue::Json(value) => {
            builder.push_bind(value.clone());
        }
        // 写入路径已在 `validate_data_changes` 拒绝截断值；这里只会被筛选条件用到，
        // 按可见预览比较即可，不会落库。
        CellValue::TruncatedText { preview, .. } => {
            builder.push_bind(preview.clone());
        }
        // 大字段同样只有预览；写入前已被 `validate_data_changes` 拒绝。
        CellValue::LargeValue(large) => {
            builder.push_bind(large.preview.clone());
        }
        CellValue::Bytes(value) => {
            builder.push_bind(value.clone());
        }
        CellValue::BinarySummary(_) => {
            builder.push_bind(Option::<Vec<u8>>::None);
        }
    }
}

fn push_sqlite_bind(builder: &mut QueryBuilder<'_, Sqlite>, value: &CellValue) {
    match value {
        CellValue::Null => {
            builder.push_bind(Option::<String>::None);
        }
        CellValue::Bool(value) => {
            builder.push_bind(*value);
        }
        CellValue::I64(value) => {
            builder.push_bind(*value);
        }
        CellValue::F64(value) => {
            builder.push_bind(*value);
        }
        CellValue::Text(value) | CellValue::Json(value) => {
            builder.push_bind(value.clone());
        }
        // 同 `push_mysql_bind`：写入前已拒绝截断值，此处仅供筛选比较。
        CellValue::TruncatedText { preview, .. } => {
            builder.push_bind(preview.clone());
        }
        CellValue::LargeValue(large) => {
            builder.push_bind(large.preview.clone());
        }
        CellValue::Bytes(value) => {
            builder.push_bind(value.clone());
        }
        CellValue::BinarySummary(_) => {
            builder.push_bind(Option::<Vec<u8>>::None);
        }
    }
}

fn push_mysql_identity_where(
    builder: &mut QueryBuilder<'_, MySql>,
    identity: &fluxdb_core::RowIdentity,
    columns: &[Column],
) -> fluxdb_core::Result<()> {
    push_identity_where(
        builder,
        identity,
        columns,
        |value| fluxdb_core::MySqlDialect.quote_identifier(value),
        push_mysql_bind,
    )
}

fn push_sqlite_identity_where(
    builder: &mut QueryBuilder<'_, Sqlite>,
    identity: &fluxdb_core::RowIdentity,
    columns: &[Column],
) -> fluxdb_core::Result<()> {
    push_identity_where(
        builder,
        identity,
        columns,
        |value| fluxdb_core::SqliteDialect.quote_identifier(value),
        push_sqlite_bind,
    )
}

fn push_identity_where<DB>(
    builder: &mut QueryBuilder<'_, DB>,
    identity: &fluxdb_core::RowIdentity,
    columns: &[Column],
    quote_identifier: fn(&str) -> String,
    push_bind: fn(&mut QueryBuilder<'_, DB>, &CellValue),
) -> fluxdb_core::Result<()>
where
    DB: sqlx::Database,
{
    validate_identity(identity, columns)?;
    builder.push(" WHERE ");

    for (index, (column, value)) in identity.values.iter().enumerate() {
        if index > 0 {
            builder.push(" AND ");
        }
        builder.push(quote_identifier(column));
        if matches!(value, CellValue::Null) {
            builder.push(" IS NULL");
        } else {
            builder.push(" = ");
            push_bind(builder, value);
        }
    }

    Ok(())
}

fn mysql_create_database_sql(request: &CreateDatabaseRequest) -> fluxdb_core::Result<String> {
    let database = request.name.trim();
    if database.is_empty() {
        return Err(Error::new(ErrorKind::Query, "数据库名称不能为空"));
    }

    let charset = request.charset.trim();
    if charset.is_empty() {
        return Err(Error::new(ErrorKind::Query, "字符集不能为空"));
    }
    if !is_mysql_option_name(charset) {
        return Err(Error::new(ErrorKind::Query, "字符集名称不合法"));
    }

    let collation = request.collation.trim();
    if collation.is_empty() {
        return Err(Error::new(ErrorKind::Query, "排序规则不能为空"));
    }
    if !is_mysql_option_name(collation) {
        return Err(Error::new(ErrorKind::Query, "排序规则名称不合法"));
    }

    Ok(format!(
        "CREATE DATABASE {} DEFAULT CHARACTER SET {} COLLATE {}",
        fluxdb_core::MySqlDialect.quote_identifier(database),
        charset,
        collation
    ))
}

fn mysql_delete_database_sql(database: &str) -> fluxdb_core::Result<String> {
    let database = database.trim();
    if database.is_empty() {
        return Err(Error::new(ErrorKind::Query, "数据库名称不能为空"));
    }

    Ok(format!("DROP DATABASE {}", fluxdb_core::MySqlDialect.quote_identifier(database)))
}

fn is_mysql_option_name(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn mysql_create_database(
    config: &ConnectionConfig,
    request: &CreateDatabaseRequest,
) -> fluxdb_core::Result<()> {
    if !is_mysql_protocol_kind(config.kind) {
        return Err(Error::new(ErrorKind::Connection, "连接类型不支持新建数据库"));
    }
    if config.id != request.connection_id {
        return Err(Error::new(ErrorKind::Connection, "连接不匹配"));
    }

    let sql = mysql_create_database_sql(request)?;
    let mut server_config = config.clone();
    if let Endpoint::Tcp { database, .. } = &mut server_config.endpoint {
        *database = None;
    }
    let url = mysql_connection_url(&server_config)?;
    let options = url
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let connect = tokio::time::timeout(Duration::from_secs(5), options.connect()).await;
        let mut connection = match connect {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => return Err(mysql_error(error)),
            Err(_) => return Err(Error::new(ErrorKind::Connection, "连接超时")),
        };

        sqlx::query(&sql)
            .execute(&mut connection)
            .await
            .map_err(mysql_error)?;
        connection.close().await.map_err(mysql_error)
    })
}

fn mysql_delete_database(
    config: &ConnectionConfig,
    connection_id: ConnectionId,
    database: &str,
) -> fluxdb_core::Result<()> {
    if !is_mysql_protocol_kind(config.kind) {
        return Err(Error::new(ErrorKind::Connection, "连接类型不支持删除数据库"));
    }
    if config.id != connection_id {
        return Err(Error::new(ErrorKind::Connection, "连接不匹配"));
    }

    let sql = mysql_delete_database_sql(database)?;
    let mut server_config = config.clone();
    if let Endpoint::Tcp { database, .. } = &mut server_config.endpoint {
        *database = None;
    }
    let url = mysql_connection_url(&server_config)?;
    let options = url
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let connect = tokio::time::timeout(Duration::from_secs(5), options.connect()).await;
        let mut connection = match connect {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => return Err(mysql_error(error)),
            Err(_) => return Err(Error::new(ErrorKind::Connection, "连接超时")),
        };

        sqlx::query(&sql)
            .execute(&mut connection)
            .await
            .map_err(mysql_error)?;
        connection.close().await.map_err(mysql_error)
    })
}

fn sqlite_quote_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod truncated_write_guard_tests {
    use super::reject_truncated_identities;
    use super::reject_truncated_values;
    use fluxdb_core::{CellUpdate, CellValue, DataChangeSet, ObjectKind, ObjectPath, Row, RowIdentity, RowUpdate};
    use std::collections::BTreeMap;

    fn changes() -> DataChangeSet {
        DataChangeSet {
            object: ObjectPath {
                connection_id: fluxdb_core::ConnectionId(1),
                database: None,
                schema: None,
                name: "CLOB_SAMPLES".to_string(),
                kind: ObjectKind::Table,
            },
            inserts: Vec::new(),
            updates: Vec::new(),
            deletes: Vec::new(),
            insert_intents: None,
        }
    }

    fn truncated() -> CellValue {
        CellValue::TruncatedText {
            preview: "前缀".to_string(),
            total_chars: 100_000,
        }
    }

    fn identity() -> RowIdentity {
        RowIdentity {
            values: BTreeMap::from([("ID".to_string(), CellValue::I64(1))]),
        }
    }

    /// 更新里带预览必须被拒绝：所有连接器写入口共用这一处校验。
    #[test]
    fn rejects_truncated_value_in_update() {
        let mut set = changes();
        set.updates.push(RowUpdate {
            identity: identity(),
            cells: vec![CellUpdate::new("NOTE", truncated())],
        });
        assert!(reject_truncated_values(&set).is_err());
    }

    #[test]
    fn rejects_truncated_value_in_insert() {
        let mut set = changes();
        set.inserts.push(Row {
            values: vec![CellValue::I64(1), truncated()],
        });
        assert!(reject_truncated_values(&set).is_err());
    }

    /// 完整文本（含恰好等于预览上限的内容）必须照常放行，不能误伤正常写入。
    #[test]
    fn allows_complete_text() {
        let mut set = changes();
        set.updates.push(RowUpdate {
            identity: identity(),
            cells: vec![CellUpdate::new("NOTE", CellValue::Text("完整内容".to_string()))],
        });
        assert!(reject_truncated_values(&set).is_ok());
    }

    fn large_value() -> CellValue {
        CellValue::LargeValue(fluxdb_core::LargeCellValue {
            content_id: "content-1".to_string(),
            kind: fluxdb_core::LargeValueKind::Text,
            byte_length: Some(1_000_000),
            preview: "前 4000 字符".to_string(),
            preview_truncated: true,
        })
    }

    /// 无主键表的整行身份里可能带大字段预览：按预览拼 WHERE 会命中 0 行，
    /// 提交却报成功。必须在提交前拒绝并提示补主键/唯一键。
    #[test]
    fn rejects_large_value_in_update_identity() {
        let mut set = changes();
        set.updates.push(RowUpdate {
            identity: RowIdentity {
                values: BTreeMap::from([
                    ("ID".to_string(), CellValue::I64(1)),
                    ("NOTE".to_string(), large_value()),
                ]),
            },
            cells: vec![CellUpdate::new("NAME", CellValue::Text("改".to_string()))],
        });
        let error = reject_truncated_identities(&set).expect_err("预览身份必须被拒绝");
        assert!(error.message.contains("主键或唯一键"));
    }

    #[test]
    fn rejects_truncated_text_in_delete_identity() {
        let mut set = changes();
        set.deletes.push(RowIdentity {
            values: BTreeMap::from([
                ("ID".to_string(), CellValue::I64(1)),
                ("NOTE".to_string(), truncated()),
            ]),
        });
        assert!(reject_truncated_identities(&set).is_err());
    }

    /// 完整值身份照常放行，不能误伤普通行的删除/更新。
    #[test]
    fn allows_complete_identity() {
        let mut set = changes();
        set.deletes.push(identity());
        set.updates.push(RowUpdate {
            identity: identity(),
            cells: vec![CellUpdate::new("NAME", CellValue::Text("改".to_string()))],
        });
        assert!(reject_truncated_identities(&set).is_ok());
    }
}
