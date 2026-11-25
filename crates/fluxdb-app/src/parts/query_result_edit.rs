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
        pagination: Pagination { offset: 0, limit },
        load_generation: 0,
        changes: None,
        editing_cell: None,
        cell_detail_panel: CellDetailPanelState::default(),
        table_info: TableInfoState::default(),
        loading: false,
        submitting: false,
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
    for keyword in [
        "distinct",
        "join",
        "group",
        "having",
        "union",
        "intersect",
        "except",
    ] {
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

    // 二段名的含义随方言不同（§8.4/R30）：PG/SQL Server/Oracle 是 schema.table，
    // MySQL/TiDB/SQLite 是 database.table。三段名统一视为 database.schema.table。
    // Oracle 的 schema 就是对象 Owner，两段名 `OWNER.TABLE` 同样按 schema 解析。
    let (database, schema, name) = query_result_object_parts(&parts, request, kind)?;

    Some(ObjectPath {
        connection_id: request.connection_id,
        database,
        schema,
        name,
        kind: ObjectKind::Table,
    })
}

/// 把 SQL 里解析出的对象名各段归一成 `(database, schema, table)`。
///
/// 各段含义按方言不同：PG/SQL Server/Oracle/达梦是 schema.table，MySQL/TiDB/SQLite
/// 是 database.table；三段名统一视为 database.schema.table。未限定的库/schema 回落到
/// 当前查询请求的作用域。
fn query_result_object_parts(
    parts: &[String],
    request: &QueryRequest,
    kind: DatabaseKind,
) -> Option<(Option<String>, Option<String>, String)> {
    match (kind, parts) {
        (
            DatabaseKind::Postgres
            | DatabaseKind::Kingbase
            | DatabaseKind::Dameng
            | DatabaseKind::SqlServer
            | DatabaseKind::Oracle,
            [name],
        ) => Some((request.database.clone(), None, name.clone())),
        (
            DatabaseKind::Postgres
            | DatabaseKind::Kingbase
            | DatabaseKind::Dameng
            | DatabaseKind::SqlServer
            | DatabaseKind::Oracle,
            [schema, name],
        ) => Some((request.database.clone(), Some(schema.clone()), name.clone())),
        (
            DatabaseKind::Postgres
            | DatabaseKind::Kingbase
            | DatabaseKind::Dameng
            | DatabaseKind::SqlServer
            | DatabaseKind::Oracle,
            [database, schema, name],
        ) => Some((Some(database.clone()), Some(schema.clone()), name.clone())),
        (_, [name]) => Some((request.database.clone(), None, name.clone())),
        (_, [database, name]) => Some((Some(database.clone()), None, name.clone())),
        (_, [database, schema, name]) => Some((
            Some(database.clone()),
            Some(schema.clone()),
            name.clone(),
        )),
        _ => None,
    }
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
/// MySQL/TiDB 用反引号，SQL Server 用方括号，其余方言用双引号；未加引号的名字在 PG 下
/// 折叠为小写（未加引号的 `FROM Sales` 指的是 `sales`），带引号则按原样保留大小写。
fn parse_sql_identifier(input: &str, kind: DatabaseKind) -> Option<(String, &str)> {
    let input = input.trim_start();
    if kind == DatabaseKind::SqlServer
        && let Some(rest) = input.strip_prefix('[')
    {
        let mut name = String::new();
        let mut rest = rest;
        loop {
            let end = rest.find(']')?;
            name.push_str(&rest[..end]);
            rest = &rest[end + 1..];
            if let Some(after_escape) = rest.strip_prefix(']') {
                // SQL Server 方括号标识符用 `]]` 表示字面 `]`。
                name.push(']');
                rest = after_escape;
                continue;
            }
            return Some((name, rest));
        }
    }
    let quote = match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => '`',
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
    // 未加引号的标识符：PG/达梦折叠小写，Oracle 折叠大写，其余按原样。
    let name = match kind {
        DatabaseKind::Postgres | DatabaseKind::Kingbase | DatabaseKind::Dameng => {
            raw.to_ascii_lowercase()
        }
        DatabaseKind::Oracle => raw.to_ascii_uppercase(),
        _ => raw.to_string(),
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
    let result_has_all_primary_keys =
        columns
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
        // 注释已在结果集补全阶段按引用表写入；这里拿不到注释时不要把它清空。
        if meta.comment.is_some() {
            column.comment = meta.comment;
        }
    }
    Some(())
}

/// 查询结果里回溯到的一张表；限定名已按方言归一，可直接用作去重键。
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct QueryResultTableRef {
    database: Option<String>,
    schema: Option<String>,
    table: String,
}

/// 单条查询最多补多少张表的注释。异常 SQL 不应触发大量元数据建连。
const QUERY_RESULT_COMMENT_TABLE_LIMIT: usize = 32;

/// 表函数/修饰关键字出现在 FROM/JOIN 之后时不是表名，跳过以免无谓拉元数据。
const QUERY_RESULT_NON_TABLE_KEYWORDS: &[&str] = &[
    "only",
    "lateral",
    "unnest",
    "generate_series",
    "generate_subscripts",
    "json_table",
    "values",
    "table",
];

/// 从结果集 SQL 引用的表。
///
/// 直接复用只读校验的方言解析器（`policy_for` + sqlparser `visit_relations`），
/// 能覆盖 join、逗号多表、子查询与引号标识符；解析失败时放弃补注释（软失败）。
/// CTE 名会被当作 relation 访问，但查不到元数据，忽略即可；别名/表达式列按
/// 结果列名反查，匹配不到时保持无注释。
fn query_result_referenced_tables(
    sql: &str,
    request: &QueryRequest,
    kind: DatabaseKind,
) -> Vec<QueryResultTableRef> {
    use std::ops::ControlFlow;

    let Some(policy) = policy_for(kind) else {
        return Vec::new();
    };
    let Ok(statements) = sqlparser::parser::Parser::parse_sql(policy.parser_dialect(), sql) else {
        return Vec::new();
    };
    let mut tables = BTreeSet::new();
    for statement in &statements {
        let _ = sqlparser::ast::visit_relations(statement, |name| {
            let parts = name
                .0
                .iter()
                .filter_map(|part| match part {
                    sqlparser::ast::ObjectNamePart::Identifier(identifier) => {
                        Some(identifier.value.clone())
                    }
                    sqlparser::ast::ObjectNamePart::Function(_) => None,
                })
                .collect::<Vec<_>>();
            if parts.len() == 1
                && QUERY_RESULT_NON_TABLE_KEYWORDS
                    .iter()
                    .any(|keyword| parts[0].eq_ignore_ascii_case(keyword))
            {
                return ControlFlow::<()>::Continue(());
            }
            if let Some((database, schema, table)) =
                query_result_object_parts(&parts, request, kind)
            {
                tables.insert(QueryResultTableRef {
                    database,
                    schema,
                    table,
                });
            }
            ControlFlow::<()>::Continue(())
        });
    }
    tables
        .into_iter()
        .take(QUERY_RESULT_COMMENT_TABLE_LIMIT)
        .collect()
}

/// 结果页是否还有需要补的列注释。
fn query_result_page_needs_comments(page: &DataPage) -> bool {
    page.columns.iter().any(|column| {
        !column
            .comment
            .as_deref()
            .is_some_and(|comment| !comment.trim().is_empty())
    })
}

/// 按列名回填注释：所有候选表都必须给出相同且非空的注释才写入。
///
/// 同名但注释冲突（或其中一张表没注释）说明无法确认结果列来自哪张表，宁可留空
/// 也不展示可能错误的注释；结果里已有的注释（可编辑结果从表元数据带出）不覆盖。
fn apply_query_result_column_comments(page: &mut DataPage, metadata: &[CompletionColumn]) {
    if metadata.is_empty() {
        return;
    }
    let mut comments_by_column: BTreeMap<String, Vec<Option<&str>>> = BTreeMap::new();
    for column in metadata {
        let comment = column
            .comment
            .as_deref()
            .map(str::trim)
            .filter(|comment| !comment.is_empty());
        comments_by_column
            .entry(column.name.to_ascii_lowercase())
            .or_default()
            .push(comment);
    }
    for column in &mut page.columns {
        if column
            .comment
            .as_deref()
            .is_some_and(|comment| !comment.trim().is_empty())
        {
            continue;
        }
        let Some(candidates) = comments_by_column.get(&column.name.to_ascii_lowercase()) else {
            continue;
        };
        let Some(first) = candidates.first().copied().flatten() else {
            continue;
        };
        if candidates.iter().all(|comment| *comment == Some(first)) {
            column.comment = Some(first.to_string());
        }
    }
}

/// 给查询结果列补字段注释。
///
/// 结果集列本身通常不带来源表信息（连接器只返回列名/类型），这里按语句引用的表
/// 从补全元数据回填：内存索引 → 盘上快照 → 连接器按需拉取。元数据失败或用户取消
/// 只影响注释，不影响查询结果本身。
fn enrich_query_result_column_comments(
    controller: &AppController,
    request: &QueryRequest,
    execution: &mut QueryExecutionResult,
    should_cancel: &dyn Fn() -> bool,
) {
    let Some(kind) = controller.connection_kind(request.connection_id) else {
        return;
    };
    // SQLite 没有列注释，省掉每次查询的元数据访问。
    if kind == DatabaseKind::Sqlite {
        return;
    }
    let mut page_index = 0usize;
    for summary in &execution.summaries {
        if summary.kind != QueryStatementKind::ResultSet || !summary.success {
            continue;
        }
        let Some(page) = execution.results.get_mut(page_index) else {
            break;
        };
        page_index += 1;
        if !query_result_page_needs_comments(page) {
            continue;
        }
        // EXPLAIN / SHOW / DESCRIBE 的结果列是执行计划或目录信息，不是来源表的字段；
        // 按列名回填会误导（如执行计划里的 id 被标成业务表主键注释）。
        if ["explain", "describe", "desc", "show", "use"]
            .iter()
            .any(|keyword| starts_with_sql_keyword(&summary.sql, keyword))
        {
            continue;
        }
        let tables = query_result_referenced_tables(&summary.sql, request, kind);
        if tables.is_empty() {
            continue;
        }
        let mut metadata = Vec::new();
        for table in &tables {
            if should_cancel() {
                tracing::debug!(
                    target: "fluxdb_app",
                    op = "query_result_column_comments",
                    "查询结果列注释补全已取消"
                );
                return;
            }
            match controller.documentation_table_columns(
                request.connection_id,
                table.database.as_deref(),
                table.schema.as_deref().or(request.schema.as_deref()),
                &table.table,
                should_cancel,
            ) {
                Ok(columns) => metadata.extend(columns),
                Err(error) => tracing::debug!(
                    target: "fluxdb_app",
                    op = "query_result_column_comments",
                    table = %table.table,
                    %error,
                    "查询结果列注释元数据不可用，跳过该表"
                ),
            }
        }
        apply_query_result_column_comments(page, &metadata);
    }
}

#[cfg(test)]
mod query_result_comment_tests {
    use super::*;

    fn request(database: Option<&str>, schema: Option<&str>) -> QueryRequest {
        QueryRequest {
            connection_id: ConnectionId(1),
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
            text: String::new(),
            mode: fluxdb_core::QueryMode::All,
            options: QueryExecutionOptions::default(),
            session_id: None,
        }
    }

    fn table(
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> QueryResultTableRef {
        QueryResultTableRef {
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
            table: table.to_string(),
        }
    }

    fn page(columns: Vec<Column>) -> DataPage {
        DataPage {
            columns,
            rows: Vec::new(),
            offset: 0,
            limit: 1,
            has_more: false,
            incomplete: None,
        }
    }

    fn column(name: &str, comment: Option<&str>) -> Column {
        Column {
            name: name.to_string(),
            type_name: Some("text".to_string()),
            nullable: true,
            primary_key: false,
            comment: comment.map(str::to_string),
        }
    }

    fn completion(table: &str, name: &str, comment: Option<&str>) -> CompletionColumn {
        CompletionColumn {
            database: None,
            schema: None,
            table: table.to_string(),
            name: name.to_string(),
            type_name: Some("text".to_string()),
            nullable: true,
            primary_key: false,
            comment: comment.map(str::to_string),
            stable: None,
        }
    }

    #[test]
    fn referenced_tables_cover_from_join_and_qualified_names() {
        let request = request(Some("app"), None);
        let tables = query_result_referenced_tables(
            "SELECT * FROM users u JOIN `app`.`orders` o ON o.user_id = u.id",
            &request,
            DatabaseKind::MySql,
        );
        assert_eq!(
            tables,
            vec![
                table(Some("app"), None, "orders"),
                table(Some("app"), None, "users"),
            ]
        );
    }

    #[test]
    fn referenced_tables_keep_quoted_case_and_schema() {
        let request = request(Some("app"), None);
        let tables = query_result_referenced_tables(
            r#"SELECT * FROM "Users" u JOIN public.orders o ON o.user_id = u.id"#,
            &request,
            DatabaseKind::Postgres,
        );
        assert_eq!(
            tables,
            vec![
                table(Some("app"), None, "Users"),
                table(Some("app"), Some("public"), "orders"),
            ]
        );
    }

    #[test]
    fn referenced_tables_skip_strings_and_table_functions() {
        let request = request(Some("app"), None);
        assert_eq!(
            query_result_referenced_tables("SELECT 'from users' AS label", &request, DatabaseKind::MySql),
            Vec::<QueryResultTableRef>::new()
        );
        assert_eq!(
            query_result_referenced_tables("SELECT * FROM unnest(author_ids) AS id", &request, DatabaseKind::Postgres),
            Vec::<QueryResultTableRef>::new()
        );
    }

    #[test]
    fn column_comments_require_unambiguous_metadata() {
        let mut page = page(vec![
            column("id", None),
            column("name", None),
            column("kept", Some("已有注释")),
            column("empty", None),
            column("shared", None),
        ]);
        let metadata = vec![
            completion("users", "id", Some("用户编号")),
            completion("orders", "id", Some("订单编号")),
            completion("users", "name", Some("用户名称")),
            completion("orders", "name", None),
            completion("users", "kept", Some("新注释")),
            completion("users", "empty", None),
            completion("users", "shared", Some("共享编号")),
            completion("orders", "shared", Some("共享编号")),
        ];
        apply_query_result_column_comments(&mut page, &metadata);
        assert_eq!(page.columns[0].comment, None);
        assert_eq!(page.columns[1].comment, None);
        assert_eq!(page.columns[2].comment.as_deref(), Some("已有注释"));
        assert_eq!(page.columns[3].comment, None);
        assert_eq!(page.columns[4].comment.as_deref(), Some("共享编号"));
    }

    /// 完整链路：结果集只带列名，补全时从内存补全索引按 SQL 引用表取回注释。
    #[test]
    fn enrichment_reads_comments_from_completion_index() {
        use fluxdb_core::{Endpoint, QueryExecutionSummary};

        let config = ConnectionConfig {
            id: ConnectionId(1),
            name: "test".into(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 1,
                database: None,
            },
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        };
        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![config.clone()]));
        controller
            .completion_index
            .lock()
            .expect("completion index")
            .replace_table_columns(
                config.id,
                Some("app"),
                None,
                "orders",
                vec![
                    completion("orders", "id", Some("订单编号")),
                    completion("orders", "name", Some("订单名称")),
                ],
                DatabaseKind::MySql,
            );

        let request = request(Some("app"), None);
        let mut execution = QueryExecutionResult {
            summaries: vec![QueryExecutionSummary {
                sql: "SELECT id, name FROM orders".into(),
                kind: QueryStatementKind::ResultSet,
                success: true,
                message: String::new(),
                returned_rows: 0,
                affected_rows: 0,
                elapsed_ms: 0,
            }],
            results: vec![page(vec![column("id", None), column("name", None)])],
            rollback_snapshots: Vec::new(),
        };
        enrich_query_result_column_comments(&controller, &request, &mut execution, &|| false);
        assert_eq!(execution.results[0].columns[0].comment.as_deref(), Some("订单编号"));
        assert_eq!(execution.results[0].columns[1].comment.as_deref(), Some("订单名称"));

        // EXPLAIN 的结果列不是来源表字段，不能按列名套用业务注释。
        let mut explain = QueryExecutionResult {
            summaries: vec![QueryExecutionSummary {
                sql: "EXPLAIN SELECT id FROM orders".into(),
                kind: QueryStatementKind::ResultSet,
                success: true,
                message: String::new(),
                returned_rows: 0,
                affected_rows: 0,
                elapsed_ms: 0,
            }],
            results: vec![page(vec![column("id", None)])],
            rollback_snapshots: Vec::new(),
        };
        enrich_query_result_column_comments(&controller, &request, &mut explain, &|| false);
        assert_eq!(explain.results[0].columns[0].comment, None);
    }
}
