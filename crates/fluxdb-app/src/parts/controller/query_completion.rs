/// T043：JOIN ON 场景子表外键列提升（设计 §8 权重 +25）。
/// 传入列是否为当前子表外键子列，命中则分数降低（分数越低越靠前）。
fn join_on_fk_score(score: i32, column_is_fk_child: bool) -> i32 {
    if column_is_fk_child {
        score.saturating_sub(25)
    } else {
        score
    }
}

/// T051：后台刷新 worker——取 index 中已有的（旧）表名，在线拉取这些表的列并写回 index。
/// 与同步 warm 共用同一批 free connector 查询函数，不依赖控制器 `&self`，故可由独立线程执行。
/// 返回 (刷新表数, 刷新列数)。刷新仅对成功拉取的列写回，失败时旧候选保留。
fn refresh_index_columns_in_background(
    index: &Mutex<CompletionIndex>,
    config: &ConnectionConfig,
    connection_id: ConnectionId,
    database: Option<&str>,
    schema: Option<&str>,
) -> fluxdb_core::Result<(usize, usize)> {
    // T052：按影响范围限制刷新对象。库级 dirty → 刷新整库；
    // 仅部分表 dirty（非库级）→ 只刷新那些表，避免每次 DDL 都重刷整库。
    let table_names: Vec<String> = {
        let Ok(guard) = index.lock() else {
            return Ok((0, 0));
        };
        let database_wide = guard.is_database_dirty(connection_id, database, schema);
        let dirty_tables = guard
            .dirty_table_names(connection_id, database, schema)
            .into_iter()
            .collect::<BTreeSet<_>>();
        guard
            .database_tables(connection_id, database, schema)
            .into_iter()
            .filter(|table| matches!(table.kind, ObjectKind::Table | ObjectKind::View))
            .filter(|table| {
                database_wide
                    || dirty_tables.is_empty()
                    || dirty_tables.contains(&table.name.to_ascii_lowercase())
            })
            .map(|table| table.name)
            .collect()
    };
    if table_names.is_empty() {
        return Ok((0, 0));
    }
    let columns = list_completion_columns_for_tables_for_connection_with_cancel(
        config,
        database,
        schema,
        &table_names,
        &|| false,
    )?;
    let refreshed_columns = columns.len();
    let mut by_table: BTreeMap<String, Vec<CompletionColumn>> = BTreeMap::new();
    for column in columns {
        by_table
            .entry(column.table.to_ascii_lowercase())
            .or_default()
            .push(column);
    }
    if let Ok(mut guard) = index.lock() {
        for table in &table_names {
            let table_columns = by_table
                .remove(&table.to_ascii_lowercase())
                .unwrap_or_default();
            // replace_table_columns 内部 touch_meta → 更新 last_verified_at 并清 dirty，索引转为 fresh。
            guard.replace_table_columns(
                connection_id,
                database,
                schema,
                table,
                table_columns,
                config.kind,
            );
        }
    }
    Ok((table_names.len(), refreshed_columns))
}

fn completion_expectation_label(expectation: CompletionExpectation) -> &'static str {
    match expectation {
        CompletionExpectation::Keyword => "keyword",
        CompletionExpectation::FromClause => "from",
        CompletionExpectation::Table => "table",
        CompletionExpectation::Column => "column",
        CompletionExpectation::Function => "function",
        CompletionExpectation::Procedure => "procedure",
        CompletionExpectation::Trigger => "trigger",
    }
}

fn quote_completion_insert_text(
    item: &mut QueryCompletionItem,
    kind: DatabaseKind,
    reserved: &impl Fn(&str) -> bool,
    force_quote: bool,
) {
    let is_identifier = matches!(
        item.kind,
        QueryCompletionKind::Schema
            | QueryCompletionKind::Table
            | QueryCompletionKind::View
            | QueryCompletionKind::Column
            | QueryCompletionKind::Function
            | QueryCompletionKind::Procedure
            | QueryCompletionKind::Trigger
    );
    if !is_identifier {
        return;
    }
    let quote_name = |name: &str| {
        name.split('.')
            .map(|part| {
                if force_quote {
                    quote_identifier(part, kind, |_| true)
                } else {
                    quote_identifier(part, kind, reserved)
                }
            })
            .collect::<Vec<_>>()
            .join(".")
    };
    if item.kind == QueryCompletionKind::Schema && item.insert_text.ends_with('.') {
        let name = &item.insert_text[..item.insert_text.len() - 1];
        item.insert_text = format!("{}.", quote_name(name));
    } else if item.kind == QueryCompletionKind::Function {
        if let Some(name) = item.insert_text.strip_suffix("()") {
            item.insert_text = format!("{}()", quote_name(name));
        } else {
            item.insert_text = quote_name(&item.insert_text);
        }
    } else {
        item.insert_text = quote_name(&item.insert_text);
    }
}

impl AppController {
    fn execute_query(&self, request: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        let rollback_snapshots = self.query_history_rollback_snapshots(request)?;
        let mut execution = self.execute_query_raw(request)?;
        execution.rollback_snapshots = rollback_snapshots;
        Ok(execution)
    }

    fn execute_query_raw(&self, request: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        let config = self
            .connection_config(request.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        execute_query_for_connection(config, request)
    }

    /// 按「执行单元 = 单条命令」批量执行：把输入文本切分为单条命令，
    /// 返回「每条命令一个 `CommandWorkbenchExecution`」的列表。
    fn execute_command_workbench_commands(
        &self,
        request: &CommandWorkbenchRequest,
    ) -> fluxdb_core::Result<Vec<CommandWorkbenchExecution>> {
        let connection_id = match request.target {
            CommandExecutionTarget::Redis { connection_id, .. } => connection_id,
            _ => {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "该目标暂不支持命令执行器",
                ))
            }
        };
        let config = self
            .connection_config(connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        execute_command_workbench_commands_for_connection(config, request)
    }

    pub fn execute_query_text_with_progress(
        &self,
        tab_id: TabId,
        text: String,
        options: QueryExecutionOptions,
        on_summary: &mut dyn FnMut(QueryExecutionSummary),
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        let request = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::QueryEditor(editor) => Some(QueryRequest {
                connection_id: editor.connection_id,
                database: editor.database.clone(),
                text: sql_text_for_execution(&text, fluxdb_core::Pagination::DEFAULT_LIMIT),
                mode: fluxdb_core::QueryMode::Selection,
                options,
            }),
            _ => None,
        });

        let Some(request) = request else {
            return Err(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
        };
        let rollback_snapshots = self.query_history_rollback_snapshots(&request)?;
        let config = self
            .connection_config(request.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        let mut execution =
            execute_query_for_connection_with_progress(config, &request, on_summary, should_cancel)?;
        execution.rollback_snapshots = rollback_snapshots;
        Ok(execution)
    }

    pub fn execute_query_text_for_scope_with_progress(
        &self,
        connection_id: ConnectionId,
        database: Option<String>,
        text: String,
        options: QueryExecutionOptions,
        on_summary: &mut dyn FnMut(QueryExecutionSummary),
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        let request = QueryRequest {
            connection_id,
            database,
            text: sql_text_for_execution(&text, fluxdb_core::Pagination::DEFAULT_LIMIT),
            mode: fluxdb_core::QueryMode::Selection,
            options,
        };
        let rollback_snapshots = self.query_history_rollback_snapshots(&request)?;
        let config = self
            .connection_config(request.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        let mut execution =
            execute_query_for_connection_with_progress(config, &request, on_summary, should_cancel)?;
        execution.rollback_snapshots = rollback_snapshots;
        Ok(execution)
    }

    fn query_completions(
        &self,
        editor: &QueryEditorState,
        cursor: usize,
        _explicit: bool,
    ) -> fluxdb_core::Result<QueryCompletionResult> {
        self.query_completions_with_cancel(editor, cursor, _explicit, &|| false)
    }

    fn query_completions_with_cancel(
        &self,
        editor: &QueryEditorState,
        cursor: usize,
        _explicit: bool,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<QueryCompletionResult> {
        let started = std::time::Instant::now();
        let text_bytes = editor.text.len();
        let config = self
            .connection_config(editor.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        let context = sql_completion_context(&editor.text, cursor.min(editor.text.len()), config.kind);
        let context_us = started.elapsed().as_micros() as u64;
        let dialect = sql_completion_dialect(config.kind);
        let database = editor.database.as_deref();
        // T051：stale-while-refresh——索引过期/dirty 时先用旧候选返回（下方各 provider
        // 读取的是仍存于 index 的旧数据），同时后台线程刷新，刷新成功后后续请求读到新候选。
        self.maybe_refresh_expired_index_in_background(config, editor.connection_id, database);
        tracing::debug!(
            target: "gdb_sql_perf",
            op = "completion_context",
            connection_id = ?editor.connection_id,
            text_bytes,
            cursor,
            prefix_len = context.prefix.len(),
            replace_start = context.replace_start,
            replace_end = context.replace_end,
            expectation = completion_expectation_label(context.expectation),
            qualifier = context.qualifier.as_deref().unwrap_or_default(),
            referenced_table_count = context.referenced_tables.len(),
            suggest_columns = context.suggest_columns,
            suggest_tables = context.suggest_tables,
            suggest_functions = context.suggest_functions,
        );
        let mut items = Vec::new();

        if context.suggest_tables {
            if should_cancel() {
                tracing::debug!(target: "gdb_sql_completion", op = "completion_cancel", phase = "tables");
                return Ok(QueryCompletionResult {
                    items,
                    replace_start: context.replace_start,
                    replace_end: context.replace_end,
                });
            }
            let (table_database, table_schema) = completion_namespace_scope(&context, database);
            let tables = self.indexed_completion_tables_with_cancel(
                config,
                editor.connection_id,
                table_database.as_deref(),
                table_schema.as_deref(),
                should_cancel,
            )
            .unwrap_or_else(|error| {
                tracing::warn!(
                    target: "gdb_sql_completion",
                    error = %error,
                    "table metadata unavailable; keeping local completion candidates"
                );
                Vec::new()
            });
            items.extend(table_completion_items(tables, &context.prefix));
        }

        if context.suggest_schemas {
            if should_cancel() {
                tracing::debug!(target: "gdb_sql_completion", op = "completion_cancel", phase = "schemas");
                return Ok(QueryCompletionResult {
                    items,
                    replace_start: context.replace_start,
                    replace_end: context.replace_end,
                });
            }
            let schemas = self
                .completion_schemas(config, editor.connection_id, database)
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        target: "gdb_sql_completion",
                        error = %error,
                        "schema metadata unavailable; keeping local completion candidates"
                    );
                    Vec::new()
                });
            items.extend(schema_completion_items(schemas, &context.prefix));
        }

        if context.suggest_keywords {
            let keywords = match context.expectation {
                CompletionExpectation::FromClause => &["FROM"][..],
                _ => dialect.keywords(),
            };
            items.extend(keyword_completion_items_for(keywords, &context.prefix));
            if context.create_table_context
                && matches!(config.kind, DatabaseKind::MySql | DatabaseKind::TiDb)
            {
                items.extend(keyword_completion_items_for(
                    MYSQL_CREATE_TABLE_KEYWORDS,
                    &context.prefix,
                ));
            }
            // 静态 SQL 片段（P1.8）：前缀触发（如 `sel` → SELECT），仅语句头部场景出现。
            if !matches!(context.expectation, CompletionExpectation::FromClause) {
                items.extend(snippet_completion_items(&context.prefix));
            }
        }

        if context.suggest_columns {
            items.extend(cte_column_completion_items(
                &context,
                &context.prefix,
                context.qualifier.as_deref(),
            ));
            // T082：派生表别名（`(subquery) t` 的 `t.`）——输出列来自子查询投影，
            // 不透写下层表 metadata（底层表列对 `t.` 无效）。
            let qualifier_is_derived = context.qualifier.as_ref().is_some_and(|qualifier| {
                context
                    .derived_columns
                    .keys()
                    .any(|alias| alias.eq_ignore_ascii_case(qualifier))
            });
            if qualifier_is_derived {
                items.extend(derived_column_completion_items(
                    &context,
                    &context.prefix,
                    context.qualifier.as_deref(),
                ));
            }
            let targets = completion_column_tables(&context);
            let qualifier_is_cte = context.qualifier.as_ref().is_some_and(|qualifier| {
                context
                    .cte_columns
                    .keys()
                    .any(|name| name.eq_ignore_ascii_case(qualifier))
            });
            if targets.is_empty() && context.qualifier.is_none() {
                if should_cancel() {
                    tracing::debug!(target: "gdb_sql_completion", op = "completion_cancel", phase = "database_columns");
                    return Ok(QueryCompletionResult {
                        items,
                        replace_start: context.replace_start,
                        replace_end: context.replace_end,
                    });
                }
                let columns = if self.state.settings.enable_completion_index {
                    self.indexed_database_completion_columns_with_cancel(
                        config,
                        editor.connection_id,
                        database,
                        None,
                        &context.prefix,
                        should_cancel,
                    )
                    .unwrap_or_else(|error| {
                        tracing::warn!(
                            target: "gdb_sql_completion",
                            error = %error,
                            "database column metadata unavailable; keeping local completion candidates"
                        );
                        Vec::new()
                    })
                } else {
                    Vec::new()
                };
                let ranked = columns
                    .into_iter()
                    .map(|column| {
                        rank_column_completion(
                            column,
                            &context.prefix,
                            CompletionColumnScope::DatabaseWide,
                        )
                    })
                    .collect::<Vec<_>>();
                items.extend(sort_ranked_completion_items(ranked));
            } else if !targets.is_empty() && !qualifier_is_cte && !qualifier_is_derived {
                let scope = if context.qualifier.is_some() {
                    CompletionColumnScope::AliasQualified
                } else {
                    CompletionColumnScope::ReferencedTable
                };
                // T043：JOIN ON 场景下子表外键列提升（设计权重 +25）。
                // 仅当处于 ON/JOIN 上下文才拉取外键；元数据拉取失败视为无外键，不阻塞补全。
                // 子表外键列加入提升集合：`split_qualified` 按 child 表定位，命中则加分。
                let mut fk_columns_by_qualifier: std::collections::HashMap<String, std::collections::HashSet<String>> =
                    std::collections::HashMap::new();
                if context.suggest_join_keys {
                    for (table, foreign_keys) in self.load_foreign_keys_bounded(
                        config,
                        editor.connection_id,
                        database,
                        &context.referenced_tables,
                        should_cancel,
                    ) {
                        let qualifier = table.alias.as_deref().unwrap_or(&table.name);
                        fk_columns_by_qualifier
                            .entry(qualifier.to_ascii_lowercase())
                            .or_default()
                            .extend(foreign_keys.iter().map(|fk| fk.column.to_ascii_lowercase()));
                    }
                }
                let mut ranked = Vec::new();
                for (target, columns) in self.load_completion_columns_bounded(
                    config,
                    editor.connection_id,
                    database,
                    &targets,
                    should_cancel,
                ) {
                    // 限定前缀：别名优先，其次表名，供 P2.10 重复列消歧使用。
                    let qualifier = target
                        .alias
                        .clone()
                        .unwrap_or_else(|| target.table.clone());
                    ranked.extend(columns.into_iter().map(|column| {
                        // T043：联合限定位先判子表外键列，再移动列进 rank。
                        let column_is_fk_child = fk_columns_by_qualifier
                            .get(&qualifier.to_ascii_lowercase())
                            .is_some_and(|cols| cols.contains(&column.name.to_ascii_lowercase()));
                        let mut ranked_item =
                            rank_column_completion(column, &context.prefix, scope);
                        ranked_item.source_qualifier = qualifier.clone();
                        ranked_item.score =
                            join_on_fk_score(ranked_item.score, column_is_fk_child);
                        ranked_item
                    }));
                }
                // 重复列消歧（P2.10）：跨表同名列加限定前缀，唯一列保持裸列名。
                items.extend(sort_ranked_completion_items(disambiguate_duplicate_columns(ranked)));
            }
        }

        if context.suggest_functions {
            items.extend(function_completion_items_for(dialect.functions(), &context.prefix));
            // 用户自定义函数（P1.7）：无参数 metadata 时插入 `name()`。
            if should_cancel() {
                tracing::debug!(target: "gdb_sql_completion", op = "completion_cancel", phase = "functions");
                return Ok(QueryCompletionResult {
                    items,
                    replace_start: context.replace_start,
                    replace_end: context.replace_end,
                });
            }
            let routines = self
                .completion_routines_with_cancel(
                    config,
                    editor.connection_id,
                    database,
                    None,
                    should_cancel,
                )
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        target: "gdb_sql_completion",
                        error = %error,
                        "function metadata unavailable; keeping built-in functions"
                    );
                    Vec::new()
                });
            items.extend(routine_completion_items(
                routines,
                CompletionRoutineKind::Function,
                &context.prefix,
            ));
        }

        if context.suggest_procedures {
            if should_cancel() {
                tracing::debug!(target: "gdb_sql_completion", op = "completion_cancel", phase = "procedures");
                return Ok(QueryCompletionResult {
                    items,
                    replace_start: context.replace_start,
                    replace_end: context.replace_end,
                });
            }
            let routines = self
                .completion_routines_with_cancel(
                    config,
                    editor.connection_id,
                    database,
                    None,
                    should_cancel,
                )
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        target: "gdb_sql_completion",
                        error = %error,
                        "procedure metadata unavailable; keeping local candidates"
                    );
                    Vec::new()
                });
            items.extend(routine_completion_items(
                routines,
                CompletionRoutineKind::Procedure,
                &context.prefix,
            ));
        }

        // P2.11：ORDER BY / GROUP BY 上下文提升当前 SELECT 别名候选。
        if context.suggest_select_aliases {
            items.extend(select_alias_completion_items(&context.select_aliases, &context.prefix));
        }

        // P2.12：`SELECT *` / `SELECT t.*` 提供显式列展开 snippet 候选。
        if let Some(star_qualifier) = &context.star_expansion {
            if should_cancel() {
                tracing::debug!(target: "gdb_sql_completion", op = "completion_cancel", phase = "star_expansion");
                return Ok(QueryCompletionResult {
                    items,
                    replace_start: context.replace_start,
                    replace_end: context.replace_end,
                });
            }
            items.extend(
                self.star_expansion_item(
                    config,
                    editor,
                    &context,
                    star_qualifier.as_deref(),
                    database,
                    should_cancel,
                )
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        target: "gdb_sql_completion",
                        error = %error,
                        "star expansion metadata unavailable; keeping other candidates"
                    );
                    Vec::new()
                }),
            );
        }

        if context.suggest_triggers {
            if should_cancel() {
                tracing::debug!(target: "gdb_sql_completion", op = "completion_cancel", phase = "triggers");
                return Ok(QueryCompletionResult {
                    items,
                    replace_start: context.replace_start,
                    replace_end: context.replace_end,
                });
            }
            let triggers = self
                .completion_triggers_with_cancel(
                    config,
                    editor.connection_id,
                    database,
                    None,
                    should_cancel,
                )
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        target: "gdb_sql_completion",
                        error = %error,
                        "trigger metadata unavailable; keeping local candidates"
                    );
                    Vec::new()
                });
            items.extend(triggers.into_iter().filter_map(|trigger| {
                matches_completion_prefix(&trigger.name, &context.prefix).then(|| QueryCompletionItem {
                    label: trigger.name.clone(),
                    insert_text: trigger.name,
                    kind: QueryCompletionKind::Trigger,
                    detail: trigger.table,
                    documentation: None,
                    filter_text: None,
                    sort_text: None,
                                    ..Default::default()
})
            }));
        }

        // P2.13：仅对真实外键生成高优先级 JOIN 提示，避免名称启发式误导。
        // 某张表外键元数据拉取失败视为无外键（partial completion），不阻塞其他候选。
        if context.suggest_join_keys {
            for (table, foreign_keys) in self.load_foreign_keys_bounded(
                config,
                editor.connection_id,
                database,
                &context.referenced_tables,
                should_cancel,
            ) {
                let root_prefix = table.alias.as_deref().unwrap_or(&table.name);
                items.extend(fk_join_completion_items(
                    root_prefix,
                    &foreign_keys,
                    config.kind,
                    &context.prefix,
                ));
            }
        }

        // 对按标识符插入的候选按方言做引号处理（P1.6）。复合标识符逐段处理，
        // 函数调用保留括号，避免把 `COUNT()` / `alias.column` 当成单个标识符。
        let reserved = |word: &str| dialect.keywords().iter().any(|k| k.eq_ignore_ascii_case(word));
        for item in &mut items {
            quote_completion_insert_text(
                item,
                config.kind,
                &reserved,
                context.quoted_identifier,
            );
        }

        // T014/T020：按下一步意图注入 expected-token 候选，并统一全局重排后再去重。
        // 预期候选（操作符/取值/子句关键字）在意图判定下优先级最高，恒靠前。
        let rank_start = std::time::Instant::now();
        let expected = expected_token_completion_items(&context.intent, &context.prefix);

        // T063：类型感知排序——在比较/取值/JOIN 上下文解析左操作列的已知类型，
        // 对与相邻列类型族兼容的列候选做排序提升。无类型信息时集合为空，提升恒不生效，
        // 排序与之前一致（验收：无类型行为不变；只提升不过滤）。
        let type_match_labels = self.comparison_type_match_labels(
            config,
            editor,
            database,
            &context,
            cursor,
            should_cancel,
        );
        let type_match = move |item: &QueryCompletionItem| {
            item.kind == QueryCompletionKind::Column && type_match_labels.contains(&item.label)
        };

        // T071/F004：可关闭个性化。关闭时 `RecencyFrequency::score` 恒 0，排序与
        // 确定性基线完全一致；开启后按采纳历史小幅加分，只重排不增删（不复活已过滤项）。
        let recency = self.recency.clone();
        let personal_score = move |item: &QueryCompletionItem| {
            recency.lock().map(|r| r.score(&item.label)).unwrap_or(0)
        };
        let items = globally_rank_completion_items(
            items,
            expected,
            &context.intent,
            &context.prefix,
            &type_match,
            &personal_score,
        );

        let result = QueryCompletionResult {
            replace_start: context.replace_start,
            replace_end: context.replace_end,
            items: dedupe_completion_items(items),
        };
        let rank_us = rank_start.elapsed().as_micros() as u64;
        let total_us = started.elapsed().as_micros() as u64;
        // T011：分阶段耗时基线。total = context + provider 组装 + 排序去重。
        // p50/p95 与远程计数等聚合指标需进程级采样，超出单次日志范围（见 T051/T011 记录）。
        tracing::debug!(
            target: "gdb_sql_perf",
            op = "app_completion",
            elapsed_us = total_us,
            context_us,
            provider_us = total_us.saturating_sub(context_us).saturating_sub(rank_us),
            rank_us,
            text_bytes,
            cursor,
            item_count = result.items.len(),
            prefix_len = context.prefix.len(),
            referenced_table_count = context.referenced_tables.len(),
            expectation = completion_expectation_label(context.expectation),
            top_candidates = %completion_top_candidates(&result.items, 8),
            intent = ?context.intent.action,
        );
        Ok(result)
    }

    fn load_foreign_keys_bounded(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        tables: &[ReferencedTable],
        should_cancel: &dyn Fn() -> bool,
    ) -> Vec<(ReferencedTable, Vec<ForeignKeyInfo>)> {
        const MAX_PARALLEL: usize = 4;
        let mut result = Vec::with_capacity(tables.len());
        for batch in tables.chunks(MAX_PARALLEL) {
            if should_cancel() {
                tracing::debug!(target: "gdb_sql_completion", op = "completion_cancel", phase = "foreign_keys_batch");
                break;
            }
            let batch_cancelled = should_cancel();
            std::thread::scope(|scope| {
                let handles = batch
                    .iter()
                    .cloned()
                    .map(|table| {
                        scope.spawn(move || {
                            let started = std::time::Instant::now();
                            let foreign_keys = self
                                .completion_foreign_keys_with_cancel(
                                    config,
                                    connection_id,
                                    table.database.as_deref().or(database),
                                    None,
                                    &table.name,
                                    &|| batch_cancelled,
                                )
                                .unwrap_or_else(|error| {
                                    tracing::warn!(
                                        target: "gdb_sql_completion",
                                        table = %table.name,
                                        error = %error,
                                        "foreign key metadata unavailable; keeping other join candidates"
                                    );
                                    Vec::new()
                                });
                            tracing::debug!(
                                target: "gdb_sql_completion",
                                op = "metadata_provider",
                                kind = "foreign_keys",
                                table = %table.name,
                                connection_id = ?connection_id,
                                elapsed_us = started.elapsed().as_micros() as u64,
                                item_count = foreign_keys.len(),
                            );
                            (table, foreign_keys)
                        })
                    })
                    .collect::<Vec<_>>();
                for handle in handles {
                    if should_cancel() {
                        tracing::debug!(target: "gdb_sql_completion", op = "completion_cancel", phase = "foreign_keys_join");
                        break;
                    }
                    if let Ok(value) = handle.join() {
                        result.push(value);
                    }
                }
            });
        }
        result
    }

    /// 并行加载多表列 metadata；批次大小固定，避免一次补全创建无限 provider 任务。
    fn load_completion_columns_bounded(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        targets: &[CompletionColumnTarget],
        should_cancel: &dyn Fn() -> bool,
    ) -> Vec<(CompletionColumnTarget, Vec<CompletionColumn>)> {
        const MAX_PARALLEL: usize = 4;
        let mut result = Vec::with_capacity(targets.len());
        for batch in targets.chunks(MAX_PARALLEL) {
            if should_cancel() {
                tracing::debug!(target: "gdb_sql_completion", op = "completion_cancel", phase = "columns_batch");
                break;
            }
            let batch_cancelled = should_cancel();
            std::thread::scope(|scope| {
                let handles = batch
                    .iter()
                    .cloned()
                    .map(|target| {
                        scope.spawn(move || {
                            let started = std::time::Instant::now();
                            let column_database = target.database.as_deref().or(database);
                            let columns = if self.state.settings.enable_completion_index {
                                self.indexed_table_completion_columns_with_cancel(
                                    config,
                                    connection_id,
                                    column_database,
                                    None,
                                    &target.table,
                                    &|| batch_cancelled,
                                )
                            } else {
                                self.completion_columns_with_cancel(
                                    config,
                                    connection_id,
                                    column_database,
                                    None,
                                    &target.table,
                                    &|| batch_cancelled,
                                )
                            }
                            .unwrap_or_else(|error| {
                                tracing::warn!(
                                    target: "gdb_sql_completion",
                                    table = %target.table,
                                    error = %error,
                                    "table column metadata unavailable; skipping this source"
                                );
                                Vec::new()
                            });
                            tracing::debug!(
                                target: "gdb_sql_completion",
                                op = "metadata_provider",
                                kind = "columns",
                                table = %target.table,
                                connection_id = ?connection_id,
                                elapsed_us = started.elapsed().as_micros() as u64,
                                item_count = columns.len(),
                            );
                            (target, columns)
                        })
                    })
                    .collect::<Vec<_>>();
                for handle in handles {
                    if should_cancel() {
                        tracing::debug!(target: "gdb_sql_completion", op = "completion_cancel", phase = "columns_join");
                        break;
                    }
                    if let Ok(value) = handle.join() {
                        result.push(value);
                    }
                }
            });
        }
        result
    }

    /// 生成 `SELECT *` / `SELECT t.*` 的列展开 snippet 候选（P2.12）。
    /// 仅提供显式 snippet，不自动改写用户文本；按列名跨表去重，超出上限截断。
    fn star_expansion_item(
        &self,
        config: &ConnectionConfig,
        editor: &QueryEditorState,
        context: &SqlCompletionContext,
        qualifier: Option<&str>,
        database: Option<&str>,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<QueryCompletionItem>> {
        // 解析星号对应的列来源：限定 `t.*` 取匹配表，裸 `*` 取全部引用表。
        let mut targets = completion_column_tables(context);
        if let Some(qualifier) = qualifier {
            targets.retain(|target| {
                target.table.eq_ignore_ascii_case(qualifier)
                    || target
                        .alias
                        .as_deref()
                        .is_some_and(|alias| alias.eq_ignore_ascii_case(qualifier))
            });
        }
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        // 拉取各目标列，跨表按列名（不区分大小写）去重。
        let mut seen = BTreeSet::new();
        let mut parts = Vec::new();
        'outer: for target in &targets {
            if should_cancel() {
                return Ok(Vec::new());
            }
            let column_database = target.database.as_deref().or(database);
            let columns = self
                .indexed_table_completion_columns_with_cancel(
                    config,
                    editor.connection_id,
                    column_database,
                    None,
                    &target.table,
                    should_cancel,
                )
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        target: "gdb_sql_completion",
                        table = %target.table,
                        error = %error,
                        "star expansion column metadata unavailable; skipping this source"
                    );
                    Vec::new()
                });
            let prefix = target.alias.clone().unwrap_or_else(|| target.table.clone());
            for column in columns {
                if !seen.insert(column.name.to_ascii_lowercase()) {
                    continue;
                }
                if parts.len() >= STAR_EXPANSION_COLUMN_LIMIT {
                    break 'outer;
                }
                parts.push(format!("{prefix}.{}", column.name));
            }
        }
        if should_cancel() {
            return Ok(Vec::new());
        }
        if parts.is_empty() {
            return Ok(Vec::new());
        }

        Ok(vec![QueryCompletionItem {
            label: "展开所有列 (*)".to_string(),
            insert_text: parts.join(", "),
            kind: QueryCompletionKind::Snippet,
            detail: Some(format!("{} 列", parts.len())),
            documentation: None,
            filter_text: Some("*".to_string()),
            sort_text: None,
            insert_text_format: InsertTextFormat::PlainText,
        }])
    }

    /// 类型感知排序（T063）：在比较/取值/JOIN ON 上下文解析左操作数列的已知类型，
    /// 返回与之「类型族兼容」的引用表列名集合。调用方据此为这些列候选做排序提升。
    /// 无法解析左列/类型、或非相关上下文时返回空集合——空集合下类型提升恒不生效，
    /// 排序与无类型信息时完全一致（T063 验收：无类型行为不变；只提升不过滤）。
    fn comparison_type_match_labels(
        &self,
        config: &ConnectionConfig,
        editor: &QueryEditorState,
        database: Option<&str>,
        context: &SqlCompletionContext,
        cursor: usize,
        should_cancel: &dyn Fn() -> bool,
    ) -> Vec<String> {
        use NextAction::*;
        if !matches!(
            context.intent.action,
            PredicateValue | JoinCondition | InsertValue
        ) {
            return Vec::new();
        }
        let before = &editor.text[..cursor.min(editor.text.len())];
        let Some((qualifier, left_name)) = comparison_left_operand(before) else {
            return Vec::new();
        };
        let Some(expected_type) = self.resolve_referenced_column_type(
            config,
            editor,
            database,
            context,
            qualifier.as_deref(),
            &left_name,
            should_cancel,
        ) else {
            return Vec::new();
        };
        let expected_family = sql_type_family(&expected_type);
        let mut labels = Vec::new();
        for target in completion_column_tables(context) {
            let column_database = target.database.as_deref().or(database);
            let Ok(columns) = self.indexed_table_completion_columns_with_cancel(
                config,
                editor.connection_id,
                column_database,
                None,
                &target.table,
                should_cancel,
            ) else {
                continue;
            };
            for column in columns {
                let Some(col_type) = column.type_name.as_deref().filter(|value| !value.is_empty())
                else {
                    continue;
                };
                if type_family_compatible(expected_family, sql_type_family(col_type)) {
                    labels.push(column.name.clone());
                }
            }
        }
        // 去重并稳定排序，便于测试断言。
        labels.sort();
        labels.dedup();
        labels
    }

    /// 在引用表中解析「列名 + 可选限定符」对应的列类型（T063）。限定符匹配 target 的
    /// 表名或别名；解析失败返回 None（不做推断）。
    fn resolve_referenced_column_type(
        &self,
        config: &ConnectionConfig,
        editor: &QueryEditorState,
        database: Option<&str>,
        context: &SqlCompletionContext,
        qualifier: Option<&str>,
        name: &str,
        should_cancel: &dyn Fn() -> bool,
    ) -> Option<String> {
        for target in completion_column_tables(context) {
            let matches_qualifier = match qualifier {
                Some(q) => {
                    target.table.eq_ignore_ascii_case(q)
                        || target
                            .alias
                            .as_deref()
                            .is_some_and(|alias| alias.eq_ignore_ascii_case(q))
                }
                None => true,
            };
            if !matches_qualifier {
                continue;
            }
            let column_database = target.database.as_deref().or(database);
            let Ok(columns) = self.indexed_table_completion_columns_with_cancel(
                config,
                editor.connection_id,
                column_database,
                None,
                &target.table,
                should_cancel,
            ) else {
                continue;
            };
            if let Some(column) = columns
                .into_iter()
                .find(|column| column.name.eq_ignore_ascii_case(name))
            {
                return column.type_name.filter(|value| !value.is_empty());
            }
        }
        None
    }

    fn load_persisted_completion_index(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> bool {
        let now = unix_timestamp_secs();
        if let Ok(index) = self.completion_index.lock()
            && index.has_database_index(connection_id, database, schema)
        {
            return !index.is_dirty_or_expired(connection_id, database, schema, now);
        }

        let Some(storage) = &self.completion_index_storage else {
            return false;
        };
        let Ok(Some(snapshot)) = storage.load_completion_index(config, database, schema) else {
            return false;
        };
        if snapshot.connection_id != connection_id {
            return false;
        }
        if let Ok(mut index) = self.completion_index.lock() {
            index.insert_snapshot(snapshot);
            !index.is_dirty_or_expired(connection_id, database, schema, now)
        } else {
            false
        }
    }

    fn save_persisted_completion_index(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
    ) {
        let Some(storage) = &self.completion_index_storage else {
            return;
        };
        let snapshot = self
            .completion_index
            .lock()
            .ok()
            .map(|index| index.snapshot(connection_id, database, schema, config.kind));
        let Some(snapshot) = snapshot else {
            return;
        };
        let current_signature = CompletionSnapshotSignature::from_snapshot(&snapshot);
        if let Ok(Some(persisted)) = storage.load_completion_index(config, database, schema)
            && CompletionSnapshotSignature::from_snapshot(&persisted) == current_signature
        {
            return;
        }
        let _ = storage.save_completion_index(config, database, schema, &snapshot);
    }

    fn indexed_completion_tables(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        self.indexed_completion_tables_with_cancel(config, connection_id, database, schema, &|| false)
    }

    fn indexed_completion_tables_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(index) = self.completion_index.lock() {
            let tables = index.database_tables(connection_id, database, schema);
            if !tables.is_empty() {
                return Ok(tables);
            }
        }

        let mut tables = loaded_completion_tables(&self.state, connection_id, database, schema);
        if tables.is_empty() {
            tables = self.completion_tables_with_cancel(config, connection_id, database, schema, should_cancel)?;
        }
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut index) = self.completion_index.lock() {
            index.insert_tables(connection_id, database, schema, tables.clone(), config.kind);
        }
        Ok(tables)
    }

    fn indexed_table_completion_columns_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(index) = self.completion_index.lock() {
            let columns = index.table_columns(connection_id, database, schema, table);
            if !columns.is_empty() {
                return Ok(columns);
            }
        }

        let mut columns = loaded_completion_columns(&self.state, connection_id, database, table);
        if columns.is_empty() {
            columns = self.completion_columns_with_cancel(
                config,
                connection_id,
                database,
                schema,
                table,
                should_cancel,
            )?;
        }
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut index) = self.completion_index.lock() {
            index.replace_table_columns(
                connection_id,
                database,
                schema,
                table,
                columns.clone(),
                config.kind,
            );
        }
        self.save_persisted_completion_index(config, connection_id, database, schema);
        Ok(columns)
    }

    fn indexed_database_completion_columns_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        prefix: &str,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(index) = self.completion_index.lock() {
            let columns = index.database_columns(connection_id, database, schema, prefix);
            if !columns.is_empty() {
                return Ok(columns);
            }
        }
        if self.load_persisted_completion_index(config, connection_id, database, schema)
            && let Ok(index) = self.completion_index.lock()
        {
            let columns = index.database_columns(connection_id, database, schema, prefix);
            if !columns.is_empty() {
                return Ok(columns);
            }
        }

        let tables = self.indexed_completion_tables_with_cancel(
            config,
            connection_id,
            database,
            schema,
            should_cancel,
        )?;
        let table_names = tables
            .iter()
            .filter(|table| matches!(table.kind, ObjectKind::Table | ObjectKind::View))
            .map(|table| table.name.clone())
            .collect::<Vec<_>>();
        if table_names.is_empty() {
            return Ok(Vec::new());
        }

        self.refresh_completion_index_tables_with_cancel(
            config,
            connection_id,
            database,
            schema,
            &table_names,
            should_cancel,
        )?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(index) = self.completion_index.lock() {
            let result = index.database_columns(connection_id, database, schema, prefix);
            drop(index);
            self.save_persisted_completion_index(config, connection_id, database, schema);
            return Ok(result);
        }
        Ok(Vec::new())
    }

    fn warm_completion_index(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> fluxdb_core::Result<()> {
        if !self.state.settings.enable_completion_index {
            return Ok(());
        }
        let config = self
            .connection_config(connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        if self.load_persisted_completion_index(config, connection_id, database, schema)
            && let Ok(index) = self.completion_index.lock()
            && !index.is_dirty_or_expired(connection_id, database, schema, unix_timestamp_secs())
        {
            return Ok(());
        }

        let tables = self.indexed_completion_tables(config, connection_id, database, schema)?;
        let mut table_names = tables
            .iter()
            .filter(|table| matches!(table.kind, ObjectKind::Table | ObjectKind::View))
            .map(|table| table.name.clone())
            .collect::<Vec<_>>();
        if let Ok(index) = self.completion_index.lock() {
            let dirty_tables = index.dirty_table_names(connection_id, database, schema);
            if !dirty_tables.is_empty()
                && !index.is_database_dirty(connection_id, database, schema)
            {
                let dirty_tables = dirty_tables.into_iter().collect::<BTreeSet<_>>();
                table_names.retain(|table| dirty_tables.contains(&table.to_ascii_lowercase()));
            }
        }
        if table_names.is_empty() {
            self.save_persisted_completion_index(config, connection_id, database, schema);
            return Ok(());
        }

        self.refresh_completion_index_tables(config, connection_id, database, schema, &table_names)?;
        self.save_persisted_completion_index(config, connection_id, database, schema);
        Ok(())
    }

    /// T051：stale-while-refresh 触发器。索引过期或 dirty 时启动一次性后台刷新，
    /// 当前请求不受阻塞（仍读旧候选）。通过 `CompletionIndex::begin_refresh` 去重，
    /// 避免每次按键重复 spawn 刷新线程。
    fn maybe_refresh_expired_index_in_background(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
    ) {
        if !self.state.settings.enable_completion_index {
            return;
        }
        let db_key = CompletionIndex::db_key(connection_id, database, None);
        // 原子地检查「已有数据且过期/dirty」并登记刷新中，避免并发请求重复触发。
        // 冷启动（新库尚未建索引）走同步 provider 填充，不做后台刷新。
        let should_refresh = {
            let Ok(mut index) = self.completion_index.lock() else {
                return;
            };
            if !index.has_database_index(connection_id, database, None) {
                return;
            }
            if !index.is_dirty_or_expired(connection_id, database, None, unix_timestamp_secs()) {
                return;
            }
            index.begin_refresh(db_key.clone())
        };
        if !should_refresh {
            return; // 已有刷新线程在进行
        }

        let index = Arc::clone(&self.completion_index);
        let config = config.clone();
        let database = database.map(str::to_string);
        tracing::debug!(
            target: "gdb_sql_completion",
            op = "index_refresh",
            phase = "start_background",
            connection_id = ?connection_id,
            database,
            "索引过期，后台刷新已触发，当前请求继续使用旧候选"
        );
        std::thread::spawn(move || {
            let result = refresh_index_columns_in_background(
                &index,
                &config,
                connection_id,
                database.as_deref(),
                None,
            );
            if let Ok(mut guard) = index.lock() {
                guard.end_refresh(&db_key);
            }
            match result {
                Ok((refreshed_tables, refreshed_columns)) => tracing::info!(
                    target: "gdb_sql_completion",
                    op = "index_refresh",
                    phase = "done",
                    connection_id = ?connection_id,
                    database,
                    refreshed_tables,
                    refreshed_columns,
                    "后台索引刷新完成，旧候选已被替换"
                ),
                Err(error) => tracing::warn!(
                    target: "gdb_sql_completion",
                    op = "index_refresh",
                    phase = "failed",
                    error = %error,
                    connection_id = ?connection_id,
                    database,
                    "后台索引刷新失败，保留旧候选"
                ),
            }
        });
    }

    fn refresh_completion_index_tables(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table_names: &[String],
    ) -> fluxdb_core::Result<()> {
        self.refresh_completion_index_tables_with_cancel(
            config,
            connection_id,
            database,
            schema,
            table_names,
            &|| false,
        )
    }

    fn refresh_completion_index_tables_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table_names: &[String],
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<()> {
        for chunk in table_names.chunks(COMPLETION_WARMUP_BATCH_SIZE) {
            if should_cancel() {
                return Ok(());
            }
            let chunk = chunk.to_vec();
            let columns = list_completion_columns_for_tables_for_connection_with_cancel(
                config,
                database,
                schema,
                &chunk,
                should_cancel,
            )?;
            if should_cancel() {
                return Ok(());
            }
            let mut by_table: BTreeMap<String, Vec<CompletionColumn>> = BTreeMap::new();
            for column in columns {
                by_table
                    .entry(column.table.to_ascii_lowercase())
                    .or_default()
                    .push(column);
            }
            if let Ok(mut index) = self.completion_index.lock() {
                for table in &chunk {
                    let table_columns = by_table
                        .remove(&table.to_ascii_lowercase())
                        .unwrap_or_default();
                    index.replace_table_columns(
                        connection_id,
                        database,
                        schema,
                        table,
                        table_columns,
                        config.kind,
                    );
                }
            }
        }
        Ok(())
    }

    fn completion_tables_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let key = CompletionTablesKey {
            connection_id,
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
        };
        let now = unix_timestamp_secs();
        if let Ok(cache) = self.completion_cache.lock()
            && let Some(entry) = cache.tables.get(&key)
            && entry.is_fresh(now)
        {
            tracing::debug!(
                target: "gdb_sql_completion",
                op = "metadata_cache_lookup",
                kind = "tables",
                connection_id = ?connection_id,
                cache_hit = true,
                item_count = entry.value.len(),
            );
            return Ok(entry.value.clone());
        }

        let started = std::time::Instant::now();
        let tables = list_completion_tables_for_connection_with_cancel(
            config,
            database,
            schema,
            "",
            COMPLETION_METADATA_LIMIT,
            should_cancel,
        )?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut cache) = self.completion_cache.lock() {
            cache.tables.insert(
                key,
                CompletionCacheEntry {
                    value: tables.clone(),
                    fetched_at: now,
                },
            );
        }
        tracing::debug!(
            target: "gdb_sql_completion",
            op = "metadata_refresh",
            kind = "tables",
            connection_id = ?connection_id,
            cache_hit = false,
            item_count = tables.len(),
            elapsed_us = started.elapsed().as_micros() as u64,
        );
        Ok(tables)
    }

    /// 收集 schema 候选（P1.5）：索引中已索引的 (database, schema) > 已加载对象 > 当前库名。
    fn completion_schemas(
        &self,
        _config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
    ) -> fluxdb_core::Result<Vec<(Option<String>, Option<String>)>> {
        if let Ok(index) = self.completion_index.lock() {
            let schemas = index.database_schemas(connection_id);
            if !schemas.is_empty() {
                return Ok(schemas);
            }
        }
        let mut schemas: Vec<(Option<String>, Option<String>)> = loaded_completion_tables(
            &self.state,
            connection_id,
            None,
            None,
        )
        .into_iter()
        .map(|table| (table.database, table.schema))
        .collect();
        if schemas.is_empty() {
            if let Some(database) = database {
                schemas.push((Some(database.to_string()), None));
            }
        }
        schemas.sort();
        schemas.dedup();
        Ok(schemas)
    }

    fn completion_columns(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        self.completion_columns_with_cancel(config, connection_id, database, schema, table, &|| false)
    }

    fn completion_columns_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let key = CompletionColumnsKey {
            connection_id,
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
            table: table.to_ascii_lowercase(),
        };
        let now = unix_timestamp_secs();
        if let Ok(cache) = self.completion_cache.lock()
            && let Some(entry) = cache.columns.get(&key)
            && entry.is_fresh(now)
        {
            tracing::debug!(
                target: "gdb_sql_completion",
                op = "metadata_cache_lookup",
                kind = "columns",
                connection_id = ?connection_id,
                table = %table,
                cache_hit = true,
                item_count = entry.value.len(),
            );
            return Ok(entry.value.clone());
        }

        let started = std::time::Instant::now();
        let columns = list_completion_columns_for_connection_with_cancel(
            config,
            database,
            schema,
            table,
            should_cancel,
        )?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut cache) = self.completion_cache.lock() {
            cache.columns.insert(
                key,
                CompletionCacheEntry {
                    value: columns.clone(),
                    fetched_at: now,
                },
            );
        }
        tracing::debug!(
            target: "gdb_sql_completion",
            op = "metadata_refresh",
            kind = "columns",
            connection_id = ?connection_id,
            table = %table,
            cache_hit = false,
            item_count = columns.len(),
            elapsed_us = started.elapsed().as_micros() as u64,
        );
        Ok(columns)
    }

    fn completion_routines_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let key = CompletionRoutinesKey {
            connection_id,
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
        };
        let now = unix_timestamp_secs();
        if let Ok(cache) = self.completion_cache.lock()
            && let Some(entry) = cache.routines.get(&key)
            && entry.is_fresh(now)
        {
            tracing::debug!(
                target: "gdb_sql_completion",
                op = "metadata_cache_lookup",
                kind = "routines",
                connection_id = ?connection_id,
                cache_hit = true,
                item_count = entry.value.len(),
            );
            return Ok(entry.value.clone());
        }

        let started = std::time::Instant::now();
        let routines = list_completion_routines_for_connection_with_cancel(
            config,
            database,
            schema,
            "",
            COMPLETION_METADATA_LIMIT,
            should_cancel,
        )?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut cache) = self.completion_cache.lock() {
            cache.routines.insert(
                key,
                CompletionCacheEntry {
                    value: routines.clone(),
                    fetched_at: now,
                },
            );
        }
        tracing::debug!(
            target: "gdb_sql_completion",
            op = "metadata_refresh",
            kind = "routines",
            connection_id = ?connection_id,
            cache_hit = false,
            item_count = routines.len(),
            elapsed_us = started.elapsed().as_micros() as u64,
        );
        Ok(routines)
    }

    fn completion_triggers_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let key = CompletionTriggersKey {
            connection_id,
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
        };
        let now = unix_timestamp_secs();
        if let Ok(cache) = self.completion_cache.lock()
            && let Some(entry) = cache.triggers.get(&key)
            && entry.is_fresh(now)
        {
            tracing::debug!(
                target: "gdb_sql_completion",
                op = "metadata_cache_lookup",
                kind = "triggers",
                connection_id = ?connection_id,
                cache_hit = true,
                item_count = entry.value.len(),
            );
            return Ok(entry.value.clone());
        }

        let started = std::time::Instant::now();
        let triggers = list_completion_triggers_for_connection_with_cancel(
            config,
            database,
            schema,
            "",
            COMPLETION_METADATA_LIMIT,
            should_cancel,
        )?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut cache) = self.completion_cache.lock() {
            cache.triggers.insert(
                key,
                CompletionCacheEntry {
                    value: triggers.clone(),
                    fetched_at: now,
                },
            );
        }
        tracing::debug!(
            target: "gdb_sql_completion",
            op = "metadata_refresh",
            kind = "triggers",
            connection_id = ?connection_id,
            cache_hit = false,
            item_count = triggers.len(),
            elapsed_us = started.elapsed().as_micros() as u64,
        );
        Ok(triggers)
    }

    /// 拉取某张表的真实外键元数据（P2.13），带注内存缓存；失败视为无外键（不阻塞补全）。
    fn completion_foreign_keys_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let key = CompletionColumnsKey {
            connection_id,
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
            table: table.to_string(),
        };
        let now = unix_timestamp_secs();
        if let Ok(cache) = self.completion_cache.lock()
            && let Some(entry) = cache.foreign_keys.get(&key)
            && entry.is_fresh(now)
        {
            tracing::debug!(
                target: "gdb_sql_completion",
                op = "metadata_cache_lookup",
                kind = "foreign_keys",
                connection_id = ?connection_id,
                table = %table,
                cache_hit = true,
                item_count = entry.value.len(),
            );
            return Ok(entry.value.clone());
        }

        let started = std::time::Instant::now();
        let foreign_keys = list_foreign_keys_for_connection_with_cancel(
            config,
            database,
            schema,
            table,
            should_cancel,
        )?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut cache) = self.completion_cache.lock() {
            cache.foreign_keys.insert(
                key,
                CompletionCacheEntry {
                    value: foreign_keys.clone(),
                    fetched_at: now,
                },
            );
        }
        tracing::debug!(
            target: "gdb_sql_completion",
            op = "metadata_refresh",
            kind = "foreign_keys",
            connection_id = ?connection_id,
            table = %table,
            cache_hit = false,
            item_count = foreign_keys.len(),
            elapsed_us = started.elapsed().as_micros() as u64,
        );
        Ok(foreign_keys)
    }

    fn clear_completion_cache(&self) {
        if let Ok(mut cache) = self.completion_cache.lock() {
            cache.tables.clear();
            cache.columns.clear();
            cache.routines.clear();
            cache.triggers.clear();
            cache.foreign_keys.clear();
        }
        if let Ok(mut index) = self.completion_index.lock() {
            *index = CompletionIndex::default();
        }
    }

    fn fail(&mut self, error: Error) -> AppEvent {
        let error = UserFacingError::from(error);
        self.state.last_error = Some(error.clone());
        AppEvent::Failed(error)
    }

    /// T054：选中项完整对象说明（懒加载）。候选构建时不调用；仅当选中的候选需要
    /// 更完整说明（如表/视图的列清单）时按需解析。数据全部来自 `CompletionIndex`，
    /// 无远程查询。对象不在索引 / 无可用描述返回 `Error`。
    ///
    /// - 表/视图 → 结构元数据：列清单「列名  类型  注释」（注释为空省略）。
    /// - 列 → 内联注释（候选携带的 `documentation`）。
    /// - 函数/过程/触发器 → 名称。
    /// - 其它（Keyword/Schema 等）→ `Error`。
    fn completion_item_documentation(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
        item: &QueryCompletionItem,
        should_cancel: &dyn Fn() -> bool,
    ) -> CompletionDocumentationState {
        match item.kind {
            QueryCompletionKind::Table | QueryCompletionKind::View => {
                let columns = self
                    .completion_index
                    .lock()
                    .map(|index| index.table_columns(connection_id, database, None, &item.label))
                    .unwrap_or_default();
                if columns.is_empty() {
                    return CompletionDocumentationState::Error("对象不在索引或没有列".to_string());
                }
                // 列清单可能较大；逐行组装时按请求取消回调快速收敛，选中项切换时
                // 旧详情不继续占用线程（latest-wins 由调用方 request id 保证）。
                let mut doc = String::new();
                for column in columns {
                    if should_cancel() {
                        return CompletionDocumentationState::Loading;
                    }
                    if !doc.is_empty() {
                        doc.push('\n');
                    }
                    doc.push_str(&column.name);
                    if let Some(type_name) = column.type_name {
                        doc.push_str("  ");
                        doc.push_str(&type_name);
                    }
                    if let Some(comment) = column.comment.filter(|text| !text.is_empty()) {
                        doc.push_str("  ");
                        doc.push_str(&comment);
                    }
                }
                CompletionDocumentationState::Ready(doc)
            }
            QueryCompletionKind::Column => match item.documentation.as_ref() {
                Some(comment) if !comment.is_empty() => {
                    CompletionDocumentationState::Ready(comment.clone())
                }
                _ => CompletionDocumentationState::Error("无列注释".to_string()),
            },
            QueryCompletionKind::Function
            | QueryCompletionKind::Procedure
            | QueryCompletionKind::Trigger
            | QueryCompletionKind::Snippet => {
                CompletionDocumentationState::Ready(item.label.clone())
            }
            _ => CompletionDocumentationState::Error("无可用文档".to_string()),
        }
    }
}

/// T054：选中项完整对象说明的懒加载状态。
///
/// `Loading` 由调用方在发起懒解析前赋值（等待索引读取 / 异步期间展示）；
/// `Ready` 携带结构元数据文本；`Error` 携带原因。
#[derive(Clone, Debug)]
pub enum CompletionDocumentationState {
    Loading,
    Ready(String),
    Error(String),
}

fn query_history_tables(sql: &str) -> Vec<String> {
    let mut tables = extract_referenced_tables(sql)
        .into_iter()
        .map(|table| table.name)
        .collect::<BTreeSet<_>>();
    if let Some(impact) = sql_ddl_impact(sql) {
        tables.extend(impact.tables);
    }
    tables.into_iter().collect()
}

fn query_history_kind(sql: &str) -> QueryHistoryKind {
    let keyword = sql_identifier_tokens(sql)
        .first()
        .map(|keyword| keyword.to_ascii_lowercase());
    match keyword.as_deref() {
        Some("select" | "with" | "show" | "describe" | "desc" | "explain") => {
            QueryHistoryKind::Query
        }
        Some("insert" | "update" | "delete" | "replace" | "merge") => QueryHistoryKind::DataChange,
        Some(
            "create" | "alter" | "drop" | "truncate" | "rename" | "grant" | "revoke" | "use",
        ) => QueryHistoryKind::SchemaChange,
        _ => QueryHistoryKind::Query,
    }
}
