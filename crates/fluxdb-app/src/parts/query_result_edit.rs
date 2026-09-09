fn query_result_editor(
    controller: &AppController,
    request: &QueryRequest,
    page: &DataPage,
) -> Option<DataEditorState> {
    let object = editable_query_object(request)?;
    let mut page = page.clone();
    apply_query_result_column_metadata(controller, &object, &mut page)?;
    let limit = page.limit;

    Some(DataEditorState {
        object,
        page: Some(page.clone()),
        original_page: Some(page),
        pagination: Pagination {
            offset: 0,
            limit,
        },
        changes: None,
        editing_cell: None,
        cell_detail_panel: CellDetailPanelState::default(),
        table_info: TableInfoState::default(),
        loading: false,
        error: None,
    })
}

fn query_result_editors(
    controller: &AppController,
    request: &QueryRequest,
    execution: &QueryExecutionResult,
) -> BTreeMap<usize, DataEditorState> {
    let mut page_index = 0usize;
    let mut editors = BTreeMap::new();
    for summary in &execution.summaries {
        if summary.kind != fluxdb_core::QueryStatementKind::ResultSet || !summary.success {
            continue;
        }
        let Some(page) = execution.results.get(page_index) else {
            break;
        };
        let statement_request = QueryRequest {
            connection_id: request.connection_id,
            database: request.database.clone(),
            text: summary.sql.clone(),
            mode: request.mode,
            options: request.options,
        };
        if let Some(editor) = query_result_editor(controller, &statement_request, page) {
            editors.insert(page_index, editor);
        }
        page_index += 1;
    }
    editors
}

fn editable_query_object(request: &QueryRequest) -> Option<ObjectPath> {
    let mut statements = sql_statement_ranges(&request.text)
        .into_iter()
        .scan(0, |start, (end, separator_len)| {
            let statement = request.text[*start..end].trim().to_string();
            *start = end + separator_len;
            Some(statement)
        })
        .filter(|statement| !statement.is_empty())
        .collect::<Vec<_>>();
    if statements.len() != 1 {
        return None;
    }

    let statement = statements.pop()?;
    if !starts_with_sql_keyword(&statement, "select") {
        return None;
    }
    for keyword in ["distinct", "join", "group", "having", "union", "intersect", "except"] {
        if has_top_level_sql_keyword(&statement, keyword) {
            return None;
        }
    }

    let from = find_top_level_sql_keyword(&statement, "from")?;
    let from_tail = statement[from + "from".len()..].trim_start();
    let (parts, rest) = parse_sql_object_name(from_tail)?;
    let rest = rest.trim_start();
    if rest.starts_with(',') || rest.starts_with('(') {
        return None;
    }

    let (database, schema, name) = match parts.as_slice() {
        [name] => (request.database.clone(), None, name.clone()),
        [database, name] => (Some(database.clone()), None, name.clone()),
        [database, schema, name] => (Some(database.clone()), Some(schema.clone()), name.clone()),
        _ => return None,
    };

    Some(ObjectPath {
        connection_id: request.connection_id,
        database,
        schema,
        name,
        kind: ObjectKind::Table,
    })
}

fn parse_sql_object_name(input: &str) -> Option<(Vec<String>, &str)> {
    let mut rest = input;
    let mut parts = Vec::new();
    loop {
        let (part, next) = parse_sql_identifier(rest)?;
        parts.push(part);
        rest = next.trim_start();
        if !rest.starts_with('.') {
            break;
        }
        rest = rest[1..].trim_start();
    }
    Some((parts, rest))
}

fn parse_sql_identifier(input: &str) -> Option<(String, &str)> {
    let input = input.trim_start();
    if let Some(rest) = input.strip_prefix('`') {
        let end = rest.find('`')?;
        return Some((rest[..end].to_string(), &rest[end + 1..]));
    }
    let end = input
        .char_indices()
        .find(|(_, ch)| !(ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '$'))
        .map(|(index, _)| index)
        .unwrap_or(input.len());
    if end == 0 {
        return None;
    }
    Some((input[..end].to_string(), &input[end..]))
}

fn apply_query_result_column_metadata(
    controller: &AppController,
    object: &ObjectPath,
    page: &mut DataPage,
) -> Option<()> {
    let mut columns = loaded_completion_columns(
        controller.state(),
        object.connection_id,
        object.database.as_deref(),
        &object.name,
    );
    if columns.is_empty() {
        let config = controller.connection_config(object.connection_id)?;
        columns = list_completion_columns_for_connection(
            config,
            object.database.as_deref(),
            object.schema.as_deref(),
            &object.name,
        )
        .ok()?;
    }
    if columns.is_empty() {
        return None;
    }

    let mut matched = Vec::with_capacity(page.columns.len());
    for column in &page.columns {
        let meta = columns
            .iter()
            .find(|meta| meta.name.eq_ignore_ascii_case(&column.name))?;
        matched.push(meta.clone());
    }

    if !columns.iter().any(|column| column.primary_key) {
        return None;
    }
    let result_has_all_primary_keys = columns
        .iter()
        .filter(|column| column.primary_key)
        .all(|primary| {
            page.columns
                .iter()
                .any(|column| column.name.eq_ignore_ascii_case(&primary.name))
        });
    if !result_has_all_primary_keys {
        return None;
    }

    for (column, meta) in page.columns.iter_mut().zip(matched) {
        column.type_name = meta.type_name;
        column.nullable = meta.nullable;
        column.primary_key = meta.primary_key;
        column.comment = meta.comment;
    }
    Some(())
}
