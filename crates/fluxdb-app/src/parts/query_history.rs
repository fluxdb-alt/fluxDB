impl AppController {
    fn record_query_execution_history(
        &mut self,
        request: &QueryRequest,
        execution: &QueryExecutionResult,
    ) {
        let executed_at_unix_secs = current_unix_secs();
        for (index, summary) in execution.summaries.iter().enumerate() {
            self.mark_query_history_completion_dirty(request, &summary.sql);
            self.state.query_history.push(QueryHistoryEntry {
                connection_id: request.connection_id,
                database: request.database.clone(),
                text: summary.sql.clone(),
                tables: query_history_tables(&summary.sql),
                kind: query_history_kind(&summary.sql),
                success: summary.success,
                summary: summary.clone(),
                executed_at_unix_secs,
                object: sql_history_object_name(&summary.sql),
                rollback_snapshot: execution
                    .rollback_snapshots
                    .get(index)
                    .cloned()
                    .flatten()
                    .filter(|_| summary.success),
            });
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
        });
    }

    fn record_data_change_history(
        &mut self,
        object: &ObjectPath,
        before_page: &DataPage,
        changes: &DataChangeSet,
    ) {
        let executed_at_unix_secs = current_unix_secs();
        self.state.query_history.extend(
            data_change_history_entries(object, before_page, changes, executed_at_unix_secs)
        );
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

fn data_change_history_entries(
    object: &ObjectPath,
    before_page: &DataPage,
    changes: &DataChangeSet,
    executed_at_unix_secs: u64,
) -> Vec<QueryHistoryEntry> {
    let mut entries = Vec::new();

    for update in &changes.updates {
        if update.cells.is_empty() {
            continue;
        }
        let assignments = update
            .cells
            .iter()
            .map(|cell| {
                format!(
                    "{} = {}",
                    sql_history_quote_ident(&cell.column),
                    sql_history_value_literal(&cell.value)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let where_clause = sql_history_row_identity(&update.identity);
        let sql = format!(
            "UPDATE {} SET {assignments} WHERE {where_clause};",
            sql_history_object_name_for_path(object)
        );
        let rollback_snapshot = data_change_update_rollback_snapshot(object, before_page, update);
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
            format!(
                "INSERT INTO {} DEFAULT VALUES;",
                sql_history_object_name_for_path(object)
            )
        } else {
            let columns = insert_values
                .iter()
                .map(|(column, _)| sql_history_quote_ident(&column.name))
                .collect::<Vec<_>>()
                .join(", ");
            let values = insert_values
                .iter()
                .map(|(_, value)| sql_history_value_literal(value))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "INSERT INTO {} ({columns}) VALUES ({values});",
                sql_history_object_name_for_path(object)
            )
        };
        let rollback_snapshot = data_change_insert_rollback_snapshot(object, before_page, row);
        entries.push(data_change_history_entry(
            object,
            sql,
            Some(rollback_snapshot),
            executed_at_unix_secs,
        ));
    }

    for identity in &changes.deletes {
        let sql = format!(
            "DELETE FROM {} WHERE {};",
            sql_history_object_name_for_path(object),
            sql_history_row_identity(identity)
        );
        let rollback_snapshot = data_change_delete_rollback_snapshot(object, before_page, identity);
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
    }
}

fn data_change_update_rollback_snapshot(
    object: &ObjectPath,
    before_page: &DataPage,
    update: &RowUpdate,
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
        table: sql_history_object_name_for_path(object),
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
        table: sql_history_object_name_for_path(object),
        identities: vec![RowIdentity { values }],
    })
}

fn data_change_delete_rollback_snapshot(
    object: &ObjectPath,
    before_page: &DataPage,
    identity: &RowIdentity,
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
        table: sql_history_object_name_for_path(object),
        columns: before_page.columns.clone(),
        rows: vec![row],
    }))
}

impl QueryHistoryEntry {
    pub fn rollback_sql(&self) -> Option<String> {
        self.rollback_snapshot
            .as_ref()
            .and_then(query_history_rollback_sql)
    }

    pub fn rollback_snapshot_summary(&self) -> Option<String> {
        self.rollback_snapshot
            .as_ref()
            .map(query_history_rollback_snapshot_summary)
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
    snapshot
        .identities
        .iter()
        .map(|identity| {
            Some(format!(
                "DELETE FROM {} WHERE {};",
                snapshot.table,
                sql_history_row_identity_for_rollback(identity)?
            ))
        })
        .collect::<Option<Vec<_>>>()
        .map(|statements| statements.join("
"))
}

fn update_rollback_sql(snapshot: &QueryUpdateRollbackSnapshot) -> Option<String> {
    let mut statements = Vec::new();
    for row in &snapshot.rows {
        let assignments = snapshot
            .changed_columns
            .iter()
            .filter_map(|column| {
                row.values.get(column).map(|value| {
                    Some(format!(
                        "{} = {}",
                        sql_history_quote_ident(column),
                        sql_history_value_literal_for_rollback(value)?
                    ))
                })
            })
            .collect::<Option<Vec<_>>>()?;
        if assignments.is_empty() {
            continue;
        }
        let where_clause = if !row.identity.values.is_empty() {
            sql_history_row_identity_for_rollback(&row.identity)?
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
                        sql_history_quote_ident(&column.name),
                        sql_history_value_literal_for_rollback(value)?,
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

fn sql_history_object_name_for_path(object: &ObjectPath) -> String {
    match object.database.as_deref() {
        Some(database) if !database.is_empty() => format!(
            "{}.{}",
            sql_history_quote_ident(database),
            sql_history_quote_ident(&object.name)
        ),
        _ => sql_history_quote_ident(&object.name),
    }
}

fn sql_history_row_identity(identity: &RowIdentity) -> String {
    if identity.values.is_empty() {
        return "1 = 0".to_string();
    }
    identity
        .values
        .iter()
        .map(|(column, value)| {
            format!(
                "{} = {}",
                sql_history_quote_ident(column),
                sql_history_value_literal(value)
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn sql_history_row_identity_for_rollback(identity: &RowIdentity) -> Option<String> {
    if identity.values.is_empty() {
        return None;
    }
    identity
        .values
        .iter()
        .map(|(column, value)| {
            Some(format!(
                "{} = {}",
                sql_history_quote_ident(column),
                sql_history_value_literal_for_rollback(value)?
            ))
        })
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join(" AND "))
}

fn sql_history_quote_ident(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}

fn sql_history_value_literal(value: &CellValue) -> String {
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
        CellValue::Text(value) | CellValue::Json(value) => {
            format!("'{}'", value.replace('\'', "''"))
        }
        CellValue::Bytes(bytes) => format!(
            "X'{}'",
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join("")
        ),
        CellValue::BinarySummary(summary) if summary.is_null => "NULL".to_string(),
        CellValue::BinarySummary(_) => "NULL".to_string(),
    }
}

fn sql_history_value_literal_for_rollback(value: &CellValue) -> Option<String> {
    match value {
        CellValue::BinarySummary(summary) if !summary.is_null => None,
        value => Some(sql_history_value_literal(value)),
    }
}
