fn query_result_editor(
    controller: &AppController,
    request: &QueryRequest,
    page: &DataPage,
) -> Option<DataEditorState> {
    let kind = controller.connection_config(request.connection_id)?.kind;
    let object = editable_query_object(request, kind)?;
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
            session_id: None,
            schema: request.schema.clone(),
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

fn editable_query_object(request: &QueryRequest, kind: DatabaseKind) -> Option<ObjectPath> {
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
    let (parts, rest) = parse_sql_object_name(from_tail, kind)?;
    let rest = rest.trim_start();
    if rest.starts_with(',') || rest.starts_with('(') {
        return None;
    }

    // 二段名的含义随方言不同（§8.4/R30）：PG 是 schema.table，MySQL/TiDB/SQLite 是
    // database.table。PG 三段视为 database.schema.table，取后两段定位（库由连接决定）。
    let (database, schema, name) = match (kind, parts.as_slice()) {
        (DatabaseKind::Postgres, [name]) => (request.database.clone(), None, name.clone()),
        (DatabaseKind::Postgres, [schema, name]) => {
            (request.database.clone(), Some(schema.clone()), name.clone())
        }
        (DatabaseKind::Postgres, [database, schema, name]) => (
            Some(database.clone()),
            Some(schema.clone()),
            name.clone(),
        ),
        (_, [name]) => (request.database.clone(), None, name.clone()),
        (_, [database, name]) => (Some(database.clone()), None, name.clone()),
        (_, [database, schema, name]) => (
            Some(database.clone()),
            Some(schema.clone()),
            name.clone(),
        ),
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

/// 解析点分对象名，返回各段名称与剩余文本（名称已按方言折叠/解转义）。
fn parse_sql_object_name(input: &str, kind: DatabaseKind) -> Option<(Vec<String>, &str)> {
    let mut rest = input;
    let mut parts = Vec::new();
    loop {
        let (part, next) = parse_sql_identifier(rest, kind)?;
        parts.push(part);
        rest = next.trim_start();
        if !rest.starts_with('.') {
            break;
        }
        rest = rest[1..].trim_start();
    }
    Some((parts, rest))
}

/// 解析单个标识符，按方言识别引号字符并解开内部转义。
///
/// MySQL/TiDB 用反引号，其余方言（PostgreSQL/SQLite）用双引号；未加引号的名字在 PG 下
/// 折叠为小写（未加引号的 `FROM Sales` 指的是 `sales`），带引号则按原样保留大小写。
fn parse_sql_identifier(input: &str, kind: DatabaseKind) -> Option<(String, &str)> {
    let input = input.trim_start();
    let quote = match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => '`',
        _ => '"',
    };
    if let Some(rest) = input.strip_prefix(quote) {
        let mut name = String::new();
        let mut rest = rest;
        loop {
            let end = rest.find(quote)?;
            name.push_str(&rest[..end]);
            rest = &rest[end + quote.len_utf8()..];
            if rest.starts_with(quote) {
                // 连续两个引号是转义后的字面引号。
                name.push(quote);
                rest = &rest[quote.len_utf8()..];
                continue;
            }
            return Some((name, rest));
        }
    }
    let end = input
        .char_indices()
        .find(|(_, ch)| !(ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '$'))
        .map(|(index, _)| index)
        .unwrap_or(input.len());
    if end == 0 {
        return None;
    }
    let raw = &input[..end];
    let name = if kind == DatabaseKind::Postgres {
        raw.to_ascii_lowercase()
    } else {
        raw.to_string()
    };
    Some((name, &input[end..]))
}

fn apply_query_result_column_metadata(
    controller: &AppController,
    object: &ObjectPath,
    page: &mut DataPage,
) -> Option<()> {
    let mut columns = loaded_completion_columns_in_schema(
        controller.state(),
        object.connection_id,
        object.database.as_deref(),
        object.schema.as_deref(),
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
