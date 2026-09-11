impl AppController {
    fn record_query_execution_history(
        &mut self,
        request: &QueryRequest,
        execution: &QueryExecutionResult,
    ) {
        let executed_at_unix_secs = current_unix_secs();
        // 一次执行内的多条语句共用同一连接，显式事务因此在批次内有效（§8.4/R11）：
        // COMMIT 前的写入先标「未提交」，COMMIT 后转「已提交」，ROLLBACK 转「已回滚」；
        // 批次结束时事务仍未提交（连接释放即被服务端回滚）也按「已回滚」标注，不谎报已提交。
        let mut transaction_open = false;
        let mut recorded_in_run = Vec::new();
        for (index, summary) in execution.summaries.iter().enumerate() {
            if history_statement_is_sensitive(&summary.sql) {
                // 敏感语句（口令/授权）不记录历史，也不留可回放的文本。
                tracing::debug!(
                    target: "gdb_query_history",
                    op = "history_record",
                    skipped = "sensitive",
                    connection_id = ?request.connection_id,
                    "敏感语句不入历史"
                );
                continue;
            }
            self.mark_query_history_completion_dirty(request, &summary.sql);
            let kind = query_history_kind(&summary.sql);
            let control = history_transaction_control(&summary.sql);
            if control == HistoryTransactionControl::Begin {
                transaction_open = true;
            }
            let transaction_state = if matches!(
                kind,
                QueryHistoryKind::DataChange | QueryHistoryKind::SchemaChange
            ) && transaction_open
                && control == HistoryTransactionControl::None
            {
                QueryHistoryTransactionState::Uncommitted
            } else {
                QueryHistoryTransactionState::Committed
            };
            let rollback_snapshot = execution
                .rollback_snapshots
                .get(index)
                .cloned()
                .flatten()
                .filter(|_| summary.success);
            self.state.query_history.push(QueryHistoryEntry {
                connection_id: request.connection_id,
                database: request.database.clone(),
                schema: request.schema.clone(),
                text: summary.sql.clone(),
                tables: query_history_tables(&summary.sql),
                kind,
                success: summary.success,
                summary: summary.clone(),
                executed_at_unix_secs,
                object: sql_history_object_name(&summary.sql),
                rollback_snapshot,
                transaction_state,
            });
            recorded_in_run.push(self.state.query_history.len() - 1);

            match control {
                HistoryTransactionControl::Commit => {
                    transaction_open = false;
                    self.settle_history_transaction(&recorded_in_run, true);
                    recorded_in_run.clear();
                }
                HistoryTransactionControl::Rollback => {
                    transaction_open = false;
                    self.settle_history_transaction(&recorded_in_run, false);
                    recorded_in_run.clear();
                }
                HistoryTransactionControl::Begin | HistoryTransactionControl::None => {}
            }
        }
        if transaction_open {
            // 未 COMMIT：连接释放时服务端回滚未提交事务，历史必须如实标注，不显示为已提交。
            self.settle_history_transaction(&recorded_in_run, false);
        }
    }

    /// 结束一次显式事务：把本次执行内已记录的写入条目按提交/回滚落定状态。
    fn settle_history_transaction(&mut self, indexes: &[usize], committed: bool) {
        for index in indexes {
            if let Some(entry) = self.state.query_history.get_mut(*index)
                && entry.transaction_state == QueryHistoryTransactionState::Uncommitted
            {
                entry.transaction_state = if committed {
                    QueryHistoryTransactionState::Committed
                } else {
                    QueryHistoryTransactionState::RolledBack
                };
            }
        }
    }

    fn query_history_rollback_snapshots(
        &self,
        request: &QueryRequest,
    ) -> fluxdb_core::Result<Vec<Option<QueryRollbackSnapshot>>> {
        split_history_statements(&request.text)
            .into_iter()
            .map(|statement| self.query_history_rollback_snapshot(request, &statement))
            .collect()
    }

    fn query_history_rollback_snapshot(
        &self,
        request: &QueryRequest,
        statement: &str,
    ) -> fluxdb_core::Result<Option<QueryRollbackSnapshot>> {
        let Some(spec) = parse_history_rollback_statement(statement) else {
            return Ok(None);
        };
        let config = self
            .connection_config(request.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        let snapshot_request = QueryRequest {
            connection_id: request.connection_id,
            database: request.database.clone(),
            session_id: None,
            schema: request.schema.clone(),
            text: spec.snapshot_sql(),
            mode: fluxdb_core::QueryMode::Selection,
            options: QueryExecutionOptions::default(),
        };
        let snapshot = self.execute_query_raw(&snapshot_request)?;
        let mut page = snapshot.results.into_iter().next().unwrap_or(DataPage {
            columns: Vec::new(),
            rows: Vec::new(),
            offset: 0,
            limit: 0,
            has_more: false,
        });
        annotate_snapshot_columns(self, config, request, spec.table_name(), &mut page);
        sanitize_snapshot_page(&mut page);

        Ok(Some(match spec {
            HistoryRollbackSpec::Update {
                table_sql,
                table_name: _,
                changed_columns,
                where_clause,
            } => {
                let primary_key_indexes = page
                    .columns
                    .iter()
                    .enumerate()
                    .filter_map(|(index, column)| column.primary_key.then_some(index))
                    .collect::<Vec<_>>();
                QueryRollbackSnapshot::Update(QueryUpdateRollbackSnapshot {
                    db_kind: Some(config.kind),
                    table: table_sql,
                    columns: page.columns.clone(),
                    changed_columns: changed_columns.clone(),
                    rows: page
                        .rows
                        .iter()
                        .map(|row| QueryRollbackRowSnapshot {
                            identity: snapshot_row_identity(
                                &page.columns,
                                row,
                                &primary_key_indexes,
                            ),
                            values: snapshot_update_values(&page.columns, row, &changed_columns),
                        })
                        .collect(),
                    fallback_where: Some(where_clause),
                })
            }
            HistoryRollbackSpec::Delete {
                table_sql,
                table_name: _,
                where_clause: _,
            } => QueryRollbackSnapshot::Delete(QueryDeleteRollbackSnapshot {
                db_kind: Some(config.kind),
                table: table_sql,
                columns: page.columns,
                rows: page.rows,
            }),
        }))
    }

    fn record_failed_query_history(&mut self, request: &QueryRequest) {
        self.mark_query_history_completion_dirty(request, &request.text);
        self.state.query_history.push(QueryHistoryEntry {
            connection_id: request.connection_id,
            database: request.database.clone(),
            schema: request.schema.clone(),
            text: request.text.clone(),
            tables: query_history_tables(&request.text),
            kind: query_history_kind(&request.text),
            success: false,
            summary: QueryExecutionSummary {
                sql: request.text.clone(),
                kind: QueryStatementKind::Command,
                success: false,
                message: "执行失败".to_string(),
                returned_rows: 0,
                affected_rows: 0,
                elapsed_ms: 0,
            },
            executed_at_unix_secs: current_unix_secs(),
            object: sql_history_object_name(&request.text),
            rollback_snapshot: None,
            // 失败语句没有落定的写入，按已提交（无写入）处理。
            transaction_state: QueryHistoryTransactionState::Committed,
        });
    }

    fn record_data_change_history(
        &mut self,
        object: &ObjectPath,
        before_page: &DataPage,
        changes: &DataChangeSet,
    ) {
        let executed_at_unix_secs = current_unix_secs();
        // 补偿 SQL 按连接方言生成（PG 双引号/bytea/精确十进制），历史记录里保存该方言。
        let kind = self
            .connection_config(object.connection_id)
            .map(|config| config.kind)
            .unwrap_or(DatabaseKind::MySql);
        self.state.query_history.extend(data_change_history_entries(
            object,
            before_page,
            changes,
            executed_at_unix_secs,
            kind,
        ));
    }

    fn mark_query_history_completion_dirty(&mut self, request: &QueryRequest, sql: &str) {
        let Some(impact) = sql_ddl_impact(sql) else {
            return;
        };
        let Ok(mut index) = self.completion_index.lock() else {
            return;
        };
        if impact.database_wide || impact.tables.is_empty() {
            index.mark_dirty(request.connection_id, request.database.as_deref(), None);
        } else {
            for table in impact.tables {
                index.mark_table_dirty(
                    request.connection_id,
                    request.database.as_deref(),
                    None,
                    &table,
                );
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum HistoryRollbackSpec {
    Update {
        table_sql: String,
        table_name: String,
        changed_columns: Vec<String>,
        where_clause: String,
    },
    Delete {
        table_sql: String,
        table_name: String,
        where_clause: String,
    },
}

impl HistoryRollbackSpec {
    fn table_name(&self) -> &str {
        match self {
            HistoryRollbackSpec::Update { table_name, .. }
            | HistoryRollbackSpec::Delete { table_name, .. } => table_name,
        }
    }

    fn snapshot_sql(&self) -> String {
        match self {
            HistoryRollbackSpec::Update {
                table_sql,
                where_clause,
                ..
            }
            | HistoryRollbackSpec::Delete {
                table_sql,
                where_clause,
                ..
            } => format!("SELECT * FROM {table_sql} WHERE {where_clause}"),
        }
    }
}

fn split_history_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut start = 0usize;
    for (end, separator_len) in sql_statement_ranges(sql) {
        let statement = sql[start..end].trim();
        if !statement.is_empty() {
            statements.push(statement.to_string());
        }
        start = end + separator_len;
    }
    if statements.is_empty() {
        let statement = sql.trim();
        if !statement.is_empty() {
            statements.push(statement.to_string());
        }
    }
    statements
}

fn parse_history_rollback_statement(statement: &str) -> Option<HistoryRollbackSpec> {
    parse_history_update_statement(statement).or_else(|| parse_history_delete_statement(statement))
}

fn parse_history_update_statement(statement: &str) -> Option<HistoryRollbackSpec> {
    let statement = statement.trim().trim_end_matches(';').trim();
    if !starts_with_sql_keyword(statement, "update") {
        return None;
    }
    let set_start = find_top_level_sql_keyword(statement, "set")?;
    let where_start = find_top_level_sql_keyword(statement, "where")?;
    if where_start <= set_start {
        return None;
    }
    let table_sql = statement["update".len()..set_start].trim();
    if !is_simple_history_table(table_sql) {
        return None;
    }
    let set_clause = statement[set_start + "set".len()..where_start].trim();
    let where_clause = statement[where_start + "where".len()..].trim();
    if set_clause.is_empty() || !is_safe_history_where_clause(where_clause) {
        return None;
    }
    let changed_columns = split_top_level_commas(set_clause)
        .into_iter()
        .filter_map(|assignment| {
            let equals_index = find_history_char(assignment, '=')?;
            normalize_history_column_name(assignment[..equals_index].trim())
        })
        .collect::<Vec<_>>();
    if changed_columns.is_empty() {
        return None;
    }
    Some(HistoryRollbackSpec::Update {
        table_name: normalize_history_table_name(table_sql),
        table_sql: table_sql.to_string(),
        changed_columns,
        where_clause: where_clause.to_string(),
    })
}

fn parse_history_delete_statement(statement: &str) -> Option<HistoryRollbackSpec> {
    let statement = statement.trim().trim_end_matches(';').trim();
    if !starts_with_sql_keyword(statement, "delete") {
        return None;
    }
    let from = find_top_level_sql_keyword(statement, "from")?;
    let where_start = find_top_level_sql_keyword(statement, "where")?;
    if where_start <= from {
        return None;
    }

    let table_sql = statement[from + "from".len()..where_start].trim();
    if !is_simple_history_table(table_sql) || table_sql.eq_ignore_ascii_case("using") {
        return None;
    }

    let where_clause = statement[where_start + "where".len()..].trim();
    if !is_safe_history_where_clause(where_clause) {
        return None;
    }

    Some(HistoryRollbackSpec::Delete {
        table_name: normalize_history_table_name(table_sql),
        table_sql: table_sql.to_string(),
        where_clause: where_clause.to_string(),
    })
}

fn is_simple_history_table(table_sql: &str) -> bool {
    !table_sql.is_empty() && !table_sql.contains(',') && table_sql.split_whitespace().count() == 1
}

fn is_safe_history_where_clause(where_clause: &str) -> bool {
    !where_clause.is_empty()
        && find_top_level_sql_keyword(where_clause, "returning").is_none()
        && find_top_level_sql_keyword(where_clause, "using").is_none()
}

fn find_history_char(value: &str, needle: char) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut chars = value.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if let Some(active_quote) = quote {
            if ch == '\\' {
                chars.next();
                continue;
            }
            if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' | '`' => quote = Some(ch),
            ch if ch == needle => return Some(index),
            _ => {}
        }
    }
    None
}

fn normalize_history_table_name(table_sql: &str) -> String {
    table_sql
        .split('.')
        .next_back()
        .map(normalize_history_ident_part)
        .unwrap_or_else(|| table_sql.trim().to_string())
}

fn normalize_history_column_name(column_sql: &str) -> Option<String> {
    column_sql
        .split('.')
        .next_back()
        .map(normalize_history_ident_part)
        .filter(|value| !value.is_empty())
}

fn normalize_history_ident_part(value: &str) -> String {
    value
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim()
        .to_string()
}

fn annotate_snapshot_columns(
    controller: &AppController,
    config: &ConnectionConfig,
    request: &QueryRequest,
    table: &str,
    page: &mut DataPage,
) {
    let Ok(metadata) = controller.completion_columns(
        config,
        request.connection_id,
        request.database.as_deref(),
        None,
        table,
    ) else {
        return;
    };
    for column in &mut page.columns {
        if let Some(metadata_column) = metadata
            .iter()
            .find(|metadata_column| metadata_column.name.eq_ignore_ascii_case(&column.name))
        {
            column.type_name = metadata_column
                .type_name
                .clone()
                .or_else(|| column.type_name.clone());
            column.nullable = metadata_column.nullable;
            column.primary_key = metadata_column.primary_key;
            column.comment = metadata_column.comment.clone();
        }
    }
}

fn snapshot_row_identity(columns: &[Column], row: &Row, primary_key_indexes: &[usize]) -> RowIdentity {
    let values = primary_key_indexes
        .iter()
        .filter_map(|index| {
            let column = columns.get(*index)?;
            let value = row.values.get(*index)?;
            Some((column.name.clone(), value.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    RowIdentity { values }
}

fn snapshot_update_values(
    columns: &[Column],
    row: &Row,
    changed_columns: &[String],
) -> BTreeMap<String, CellValue> {
    changed_columns
        .iter()
        .filter_map(|changed_column| {
            columns
                .iter()
                .position(|column| column.name.eq_ignore_ascii_case(changed_column))
                .and_then(|index| {
                    row.values.get(index).map(|value| {
                        (
                            changed_column.clone(),
                            history_snapshot_value(value.clone(), columns.get(index)),
                        )
                    })
                })
        })
        .collect()
}

fn sanitize_snapshot_page(page: &mut DataPage) {
    for row in &mut page.rows {
        for (index, value) in row.values.iter_mut().enumerate() {
            let column = page.columns.get(index);
            *value = history_snapshot_value(value.clone(), column);
        }
    }
}

const HISTORY_INLINE_BYTES_LIMIT: usize = 1024 * 1024;

fn history_snapshot_value(value: CellValue, column: Option<&Column>) -> CellValue {
    match value {
        CellValue::Bytes(bytes) if bytes.len() > HISTORY_INLINE_BYTES_LIMIT => {
            CellValue::BinarySummary(BinaryCellSummary {
                type_name: column
                    .and_then(|column| column.type_name.clone())
                    .unwrap_or_else(|| "binary".to_string()),
                is_null: false,
                byte_length: bytes.len() as u64,
                preview_hex: Some(bytes_preview_hex(&bytes)),
            })
        }
        value => value,
    }
}

fn current_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// 显式事务控制语句类型（用于历史事务状态）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HistoryTransactionControl {
    Begin,
    Commit,
    Rollback,
    None,
}

/// 识别显式事务控制语句。`ROLLBACK TO SAVEPOINT` 不算结束事务（事务仍开着）。
fn history_transaction_control(sql: &str) -> HistoryTransactionControl {
    let tokens = sql_identifier_tokens(sql)
        .into_iter()
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    match tokens.first().map(String::as_str) {
        Some("begin") => HistoryTransactionControl::Begin,
        Some("start") if tokens.get(1).is_some_and(|token| token == "transaction") => {
            HistoryTransactionControl::Begin
        }
        Some("commit") => HistoryTransactionControl::Commit,
        Some("rollback") => {
            // `ROLLBACK TO [SAVEPOINT] x` 只回退到保存点，事务继续。
            if tokens.get(1).is_some_and(|token| token == "to") {
                HistoryTransactionControl::None
            } else {
                HistoryTransactionControl::Rollback
            }
        }
        _ => HistoryTransactionControl::None,
    }
}

/// 敏感语句不入历史（§8.4/R11）：口令/角色/授权类语句既含凭据也不能安全回放。
fn history_statement_is_sensitive(sql: &str) -> bool {
    let tokens = sql_identifier_tokens(sql)
        .into_iter()
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let first = tokens.first().map(String::as_str);
    let second = tokens.get(1).map(String::as_str);
    let third = tokens.get(2).map(String::as_str);
    match (first, second) {
        (Some("set"), Some("password")) => true,
        (Some("create" | "alter" | "drop"), Some("user" | "role" | "login" | "group")) => true,
        (Some("grant" | "revoke"), _) => true,
        // `ALTER USER ... IDENTIFIED BY ...` / `CREATE USER ... PASSWORD ...` 等口令行。
        _ => third.is_some_and(|_| {
            tokens
                .iter()
                .any(|token| matches!(token.as_str(), "identified" | "password" | "passwd"))
                && matches!(first, Some("create" | "alter" | "set" | "update"))
        }),
    }
}

fn data_change_history_entries(
    object: &ObjectPath,
    before_page: &DataPage,
    changes: &DataChangeSet,
    executed_at_unix_secs: u64,
    kind: DatabaseKind,
) -> Vec<QueryHistoryEntry> {
    let mut entries = Vec::new();
    let table_name = sql_history_object_name_for_path(object, kind);

    for update in &changes.updates {
        if update.cells.is_empty() {
            continue;
        }
        let assignments = update
            .cells
            .iter()
            .map(|cell| {
                let column_type = before_page
                    .columns
                    .iter()
                    .find(|column| column.name == cell.column)
                    .and_then(|column| column.type_name.as_deref());
                format!(
                    "{} = {}",
                    sql_history_quote_ident_for(&cell.column, kind),
                    sql_history_value_literal_for_type(&cell.value, kind, column_type)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let where_clause = sql_history_row_identity_for(update.identity.values.iter(), kind);
        let sql = format!("UPDATE {table_name} SET {assignments} WHERE {where_clause};");
        let rollback_snapshot =
            data_change_update_rollback_snapshot(object, before_page, update, kind);
        entries.push(data_change_history_entry(
            object,
            sql,
            rollback_snapshot,
            executed_at_unix_secs,
        ));
    }

    for row in &changes.inserts {
        let insert_values = before_page
            .columns
            .iter()
            .zip(row.values.iter())
            .filter(|(_, value)| !matches!(value, CellValue::Null))
            .collect::<Vec<_>>();
        let sql = if insert_values.is_empty() {
            format!("INSERT INTO {table_name} DEFAULT VALUES;")
        } else {
            let columns = insert_values
                .iter()
                .map(|(column, _)| sql_history_quote_ident_for(&column.name, kind))
                .collect::<Vec<_>>()
                .join(", ");
            let values = insert_values
                .iter()
                .map(|(column, value)| {
                    sql_history_value_literal_for_type(value, kind, column.type_name.as_deref())
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("INSERT INTO {table_name} ({columns}) VALUES ({values});")
        };
        let rollback_snapshot =
            data_change_insert_rollback_snapshot(object, before_page, row, kind);
        entries.push(data_change_history_entry(
            object,
            sql,
            Some(rollback_snapshot),
            executed_at_unix_secs,
        ));
    }

    for identity in &changes.deletes {
        let sql = format!(
            "DELETE FROM {table_name} WHERE {};",
            sql_history_row_identity_for(identity.values.iter(), kind)
        );
        let rollback_snapshot =
            data_change_delete_rollback_snapshot(object, before_page, identity, kind);
        entries.push(data_change_history_entry(
            object,
            sql,
            rollback_snapshot,
            executed_at_unix_secs,
        ));
    }

    entries
}

fn data_change_history_entry(
    object: &ObjectPath,
    sql: String,
    rollback_snapshot: Option<QueryRollbackSnapshot>,
    executed_at_unix_secs: u64,
) -> QueryHistoryEntry {
    QueryHistoryEntry {
        connection_id: object.connection_id,
        database: object.database.clone(),
        schema: object.schema.clone(),
        text: sql.clone(),
        tables: vec![object.name.clone()],
        kind: QueryHistoryKind::DataChange,
        success: true,
        summary: QueryExecutionSummary {
            sql,
            kind: QueryStatementKind::Command,
            success: true,
            message: "OK".to_string(),
            returned_rows: 0,
            affected_rows: 1,
            elapsed_ms: 0,
        },
        executed_at_unix_secs,
        object: Some(object.name.clone()),
        rollback_snapshot,
        // 数据编辑器的提交是即时写入（各自自动提交），落定即已提交。
        transaction_state: QueryHistoryTransactionState::Committed,
    }
}

fn data_change_update_rollback_snapshot(
    object: &ObjectPath,
    before_page: &DataPage,
    update: &RowUpdate,
    kind: DatabaseKind,
) -> Option<QueryRollbackSnapshot> {
    let row = page_row_for_identity(before_page, &update.identity)?;
    let values = update
        .cells
        .iter()
        .filter_map(|cell| {
            before_page
                .columns
                .iter()
                .position(|column| column.name == cell.column)
                .and_then(|index| row.values.get(index))
                .map(|value| (cell.column.clone(), history_snapshot_value(value.clone(), None)))
        })
        .collect::<BTreeMap<_, _>>();
    if values.is_empty() {
        return None;
    }
    Some(QueryRollbackSnapshot::Update(QueryUpdateRollbackSnapshot {
        db_kind: Some(kind),
        table: sql_history_object_name_for_path(object, kind),
        columns: before_page.columns.clone(),
        changed_columns: update
            .cells
            .iter()
            .map(|cell| cell.column.clone())
            .collect::<Vec<_>>(),
        rows: vec![QueryRollbackRowSnapshot {
            identity: update.identity.clone(),
            values,
        }],
        fallback_where: None,
    }))
}

fn data_change_insert_rollback_snapshot(
    object: &ObjectPath,
    before_page: &DataPage,
    row: &Row,
    kind: DatabaseKind,
) -> QueryRollbackSnapshot {
    let values = before_page
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.primary_key)
        .filter_map(|(index, column)| {
            row.values
                .get(index)
                .map(|value| (column.name.clone(), value.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let values = if values.is_empty() {
        before_page
            .columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| {
                row.values
                    .get(index)
                    .filter(|value| !matches!(value, CellValue::Null))
                    .map(|value| (column.name.clone(), value.clone()))
            })
            .collect()
    } else {
        values
    };
    QueryRollbackSnapshot::Insert(QueryInsertRollbackSnapshot {
        db_kind: Some(kind),
        table: sql_history_object_name_for_path(object, kind),
        identities: vec![RowIdentity { values }],
    })
}

fn data_change_delete_rollback_snapshot(
    object: &ObjectPath,
    before_page: &DataPage,
    identity: &RowIdentity,
    kind: DatabaseKind,
) -> Option<QueryRollbackSnapshot> {
    let row = page_row_for_identity(before_page, identity)?;
    let row = Row {
        values: before_page
            .columns
            .iter()
            .zip(row.values.iter())
            .map(|(column, value)| history_snapshot_value(value.clone(), Some(column)))
            .collect(),
    };
    Some(QueryRollbackSnapshot::Delete(QueryDeleteRollbackSnapshot {
        db_kind: Some(kind),
        table: sql_history_object_name_for_path(object, kind),
        columns: before_page.columns.clone(),
        rows: vec![row],
    }))
}

impl QueryHistoryEntry {
    pub fn rollback_sql(&self) -> Option<String> {
        // 已回滚的写入没有留下任何变更：不提供补偿 SQL，避免把从未生效的改动再写一遍（§8.4/R11）。
        if self.transaction_state == QueryHistoryTransactionState::RolledBack {
            return None;
        }
        self.rollback_snapshot
            .as_ref()
            .and_then(query_history_rollback_sql)
    }

    pub fn rollback_snapshot_summary(&self) -> Option<String> {
        self.rollback_snapshot
            .as_ref()
            .map(query_history_rollback_snapshot_summary)
    }

    /// 事务状态文案；已提交时不额外标注，避免噪声。
    pub fn transaction_state_label(&self) -> Option<&'static str> {
        match self.transaction_state {
            QueryHistoryTransactionState::Committed => None,
            QueryHistoryTransactionState::Uncommitted => Some("未提交（事务进行中）"),
            QueryHistoryTransactionState::RolledBack => Some("已回滚（未提交或显式 ROLLBACK）"),
        }
    }
}

fn query_history_rollback_sql(snapshot: &QueryRollbackSnapshot) -> Option<String> {
    match snapshot {
        QueryRollbackSnapshot::Insert(snapshot) => insert_rollback_sql(snapshot),
        QueryRollbackSnapshot::Update(snapshot) => update_rollback_sql(snapshot),
        QueryRollbackSnapshot::Delete(snapshot) => delete_rollback_sql(snapshot),
    }
}

fn insert_rollback_sql(snapshot: &QueryInsertRollbackSnapshot) -> Option<String> {
    let kind = snapshot.db_kind.unwrap_or(DatabaseKind::MySql);
    snapshot
        .identities
        .iter()
        .map(|identity| {
            Some(format!(
                "DELETE FROM {} WHERE {};",
                snapshot.table,
                sql_history_row_identity_for_rollback_in(identity, kind)?
            ))
        })
        .collect::<Option<Vec<_>>>()
        .map(|statements| statements.join("
"))
}

fn update_rollback_sql(snapshot: &QueryUpdateRollbackSnapshot) -> Option<String> {
    let kind = snapshot.db_kind.unwrap_or(DatabaseKind::MySql);
    let mut statements = Vec::new();
    for row in &snapshot.rows {
        let assignments = snapshot
            .changed_columns
            .iter()
            .filter_map(|column| {
                row.values.get(column).map(|value| {
                    // 列类型用于 PG 的精确十进制/JSON 字面量渲染。
                    let column_type = snapshot
                        .columns
                        .iter()
                        .find(|meta| meta.name == *column)
                        .and_then(|meta| meta.type_name.as_deref());
                    Some(format!(
                        "{} = {}",
                        sql_history_quote_ident_for(column, kind),
                        sql_history_value_literal_for_rollback_with_type(value, kind, column_type)?
                    ))
                })
            })
            .collect::<Option<Vec<_>>>()?;
        if assignments.is_empty() {
            continue;
        }
        let where_clause = if !row.identity.values.is_empty() {
            sql_history_row_identity_for_rollback_in(&row.identity, kind)?
        } else if snapshot.rows.len() == 1 {
            snapshot.fallback_where.clone()?
        } else {
            return None;
        };
        statements.push(format!(
            "UPDATE {} SET {} WHERE {};",
            snapshot.table,
            assignments.join(", "),
            where_clause
        ));
    }
    (!statements.is_empty()).then(|| statements.join("
"))
}

fn delete_rollback_sql(snapshot: &QueryDeleteRollbackSnapshot) -> Option<String> {
    let kind = snapshot.db_kind.unwrap_or(DatabaseKind::MySql);
    snapshot
        .rows
        .iter()
        .map(|row| {
            let values = snapshot
                .columns
                .iter()
                .zip(row.values.iter())
                .filter(|(_, value)| !matches!(value, CellValue::Null))
                .map(|(column, value)| {
                    Some((
                        sql_history_quote_ident_for(&column.name, kind),
                        sql_history_value_literal_for_rollback_with_type(
                            value,
                            kind,
                            column.type_name.as_deref(),
                        )?,
                    ))
                })
                .collect::<Option<Vec<_>>>()?;
            if values.is_empty() {
                return Some(format!("INSERT INTO {} DEFAULT VALUES;", snapshot.table));
            }
            let columns = values
                .iter()
                .map(|(column, _)| column.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let literals = values
                .iter()
                .map(|(_, value)| value.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!(
                "INSERT INTO {} ({columns}) VALUES ({literals});",
                snapshot.table
            ))
        })
        .collect::<Option<Vec<_>>>()
        .map(|statements| statements.join("
"))
}

fn query_history_rollback_snapshot_summary(snapshot: &QueryRollbackSnapshot) -> String {
    match snapshot {
        QueryRollbackSnapshot::Insert(snapshot) => format!(
            "表: {}
回滚定位: {} 行插入记录",
            snapshot.table,
            snapshot.identities.len()
        ),
        QueryRollbackSnapshot::Update(snapshot) => {
            let columns = if snapshot.changed_columns.is_empty() {
                "-".to_string()
            } else {
                snapshot.changed_columns.join(", ")
            };
            let mut text = format!(
                "表: {}
变更字段: {columns}
原始行: {} 行",
                snapshot.table,
                snapshot.rows.len()
            );
            for (index, row) in snapshot.rows.iter().take(20).enumerate() {
                text.push_str(&format!(
                    "
#{} {}",
                    index + 1,
                    query_history_values_summary(&row.values)
                ));
            }
            if snapshot.rows.len() > 20 {
                text.push_str(&format!("
... 还有 {} 行", snapshot.rows.len() - 20));
            }
            text
        }
        QueryRollbackSnapshot::Delete(snapshot) => {
            let mut text = format!(
                "表: {}
列: {}
原始行: {} 行",
                snapshot.table,
                snapshot
                    .columns
                    .iter()
                    .map(|column| column.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                snapshot.rows.len()
            );
            for (index, row) in snapshot.rows.iter().take(20).enumerate() {
                let values = snapshot
                    .columns
                    .iter()
                    .zip(row.values.iter())
                    .map(|(column, value)| (column.name.clone(), value.clone()))
                    .collect::<BTreeMap<_, _>>();
                text.push_str(&format!(
                    "
#{} {}",
                    index + 1,
                    query_history_values_summary(&values)
                ));
            }
            if snapshot.rows.len() > 20 {
                text.push_str(&format!("
... 还有 {} 行", snapshot.rows.len() - 20));
            }
            text
        }
    }
}

fn query_history_values_summary(values: &BTreeMap<String, CellValue>) -> String {
    values
        .iter()
        .map(|(column, value)| format!("{column}={}", query_history_value_summary(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn query_history_value_summary(value: &CellValue) -> String {
    match value {
        CellValue::Text(value) | CellValue::Json(value) if value.len() > 120 => {
            format!("'{}...'", value.chars().take(120).collect::<String>())
        }
        CellValue::Bytes(bytes) => format!("X'{}' ({} bytes)", bytes_preview_hex(bytes), bytes.len()),
        CellValue::BinarySummary(summary) => {
            if summary.is_null {
                "NULL".to_string()
            } else {
                format!("{} [{}]", summary.type_name, summary.byte_length)
            }
        }
        _ => value.display_label(),
    }
}

fn bytes_preview_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(32)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn page_row_for_identity<'a>(page: &'a DataPage, identity: &RowIdentity) -> Option<&'a Row> {
    page.rows.iter().find(|row| {
        identity.values.iter().all(|(column_name, value)| {
            page.columns
                .iter()
                .position(|column| &column.name == column_name)
                .and_then(|index| row.values.get(index))
                .is_some_and(|row_value| row_value == value)
        })
    })
}

fn sql_history_object_name(sql: &str) -> Option<String> {
    let tokens = sql
        .split(|ch: char| ch.is_whitespace() || matches!(ch, '(' | ')' | ',' | ';'))
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let lower = tokens
        .iter()
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let index = match lower.first().map(String::as_str) {
        Some("select") => lower.iter().position(|token| token == "from"),
        Some("insert") => lower.iter().position(|token| token == "into"),
        Some("update") => Some(0),
        Some("delete") => lower.iter().position(|token| token == "from"),
        _ => None,
    }?;
    let token = if lower.first().map(String::as_str) == Some("update") {
        tokens.get(1)?
    } else {
        tokens.get(index + 1)?
    };
    Some(token.trim_matches('`').trim_matches('"').to_string())
}

/// 补偿 SQL 里的限定对象名（按方言加引号）。
///
/// PG 的限定层级是 schema.table（不带库名，库由连接决定）；MySQL/TiDB/SQLite 是
/// database.table。schema/database 未知时退化为裸表名，由连接的 search_path/默认库解析。
fn sql_history_object_name_for_path(object: &ObjectPath, kind: DatabaseKind) -> String {
    let qualifier = if kind == DatabaseKind::Postgres {
        object.schema.as_deref()
    } else {
        object.database.as_deref()
    };
    match qualifier.filter(|value| !value.is_empty()) {
        Some(qualifier) => format!(
            "{}.{}",
            sql_history_quote_ident_for(qualifier, kind),
            sql_history_quote_ident_for(&object.name, kind)
        ),
        None => sql_history_quote_ident_for(&object.name, kind),
    }
}

/// 行身份 → WHERE 子句（按方言渲染，供历史条目展示的 SQL 使用）。
fn sql_history_row_identity_for<'a>(
    pairs: impl Iterator<Item = (&'a String, &'a CellValue)>,
    kind: DatabaseKind,
) -> String {
    let clauses = pairs
        .map(|(column, value)| {
            format!(
                "{} = {}",
                sql_history_quote_ident_for(column, kind),
                sql_history_value_literal_for_type(value, kind, None)
            )
        })
        .collect::<Vec<_>>();
    if clauses.is_empty() {
        return "1 = 0".to_string();
    }
    clauses.join(" AND ")
}

/// 行身份 → WHERE 子句（按方言渲染标识符与字面量）。
fn sql_history_row_identity_for_rollback_in(
    identity: &RowIdentity,
    kind: DatabaseKind,
) -> Option<String> {
    if identity.values.is_empty() {
        return None;
    }
    identity
        .values
        .iter()
        .map(|(column, value)| {
            Some(format!(
                "{} = {}",
                sql_history_quote_ident_for(column, kind),
                sql_history_value_literal_for_rollback_with_type(value, kind, None)?
            ))
        })
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join(" AND "))
}

/// 按方言给标识符加引号（PG 用双引号，MySQL/TiDB/SQLite 用反引号）。
fn sql_history_quote_ident_for(value: &str, kind: DatabaseKind) -> String {
    if kind == DatabaseKind::Postgres {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        format!("`{}`", value.replace('`', "``"))
    }
}

/// 十六进制字节序列（bytea/BLOB 共用）。
fn sql_history_hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

/// 列类型是否属于需要按「裸数值字面量」渲染的精确十进制族。
///
/// PG 的 numeric/decimal/money 走文本保精度（见 postgres/values.rs），补偿 SQL 若把它
/// 当字符串写回会引入隐式转换与格式差异，故按数值字面量原样输出（§8.4 decimal 不失真）。
fn sql_history_numeric_type(type_name: Option<&str>) -> bool {
    let Some(type_name) = type_name else {
        return false;
    };
    let lower = type_name.trim().to_ascii_lowercase();
    let base = lower.split('(').next().unwrap_or(&lower).trim();
    matches!(
        base,
        "numeric" | "decimal" | "dec" | "money" | "smallmoney" | "int" | "integer" | "bigint"
            | "smallint" | "tinyint" | "mediumint" | "int2" | "int4" | "int8" | "double"
            | "double precision" | "real" | "float" | "float4" | "float8"
    )
}

/// 十进制文本是否可安全作为裸数值字面量输出（拒绝 NaN/Infinity/含引号等异常文本）。
fn sql_history_numeric_text(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+' | 'e' | 'E'))
        && trimmed.chars().any(|ch| ch.is_ascii_digit())
}

fn sql_history_value_literal_for_type(
    value: &CellValue,
    kind: DatabaseKind,
    type_name: Option<&str>,
) -> String {
    let postgres = kind == DatabaseKind::Postgres;
    match value {
        CellValue::Null => "NULL".to_string(),
        CellValue::Bool(value) => {
            if *value {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        CellValue::I64(value) => value.to_string(),
        CellValue::F64(value) => {
            if value.is_finite() {
                value.to_string()
            } else {
                "NULL".to_string()
            }
        }
        // PG：精确十进制文本按数值字面量输出；json/jsonb 用具名类型转换保留类型。
        CellValue::Text(value) => {
            if postgres && sql_history_numeric_type(type_name) && sql_history_numeric_text(value) {
                value.trim().to_string()
            } else if postgres
                && type_name
                    .map(|name| {
                        let lower = name.trim().to_ascii_lowercase();
                        lower == "json" || lower == "jsonb"
                    })
                    .unwrap_or(false)
            {
                format!("'{}'::{}", value.replace('\'', "''"), type_name.unwrap_or("jsonb").trim())
            } else {
                format!("'{}'", value.replace('\'', "''"))
            }
        }
        CellValue::Json(value) => {
            if postgres {
                format!(
                    "'{}'::{}",
                    value.replace('\'', "''"),
                    type_name.map(str::trim).unwrap_or("jsonb")
                )
            } else {
                format!("'{}'", value.replace('\'', "''"))
            }
        }
        CellValue::Bytes(bytes) => {
            let hex = sql_history_hex_bytes(bytes);
            if postgres {
                // PG 十六进制 bytea 字面量，显式 ::bytea 防止被当作 text 写入。
                format!("'\\x{hex}'::bytea")
            } else {
                format!("X'{hex}'")
            }
        }
        CellValue::BinarySummary(summary) if summary.is_null => "NULL".to_string(),
        CellValue::BinarySummary(_) => "NULL".to_string(),
    }
}

fn sql_history_value_literal_for_rollback_with_type(
    value: &CellValue,
    kind: DatabaseKind,
    type_name: Option<&str>,
) -> Option<String> {
    match value {
        CellValue::BinarySummary(summary) if !summary.is_null => None,
        value => Some(sql_history_value_literal_for_type(value, kind, type_name)),
    }
}

