impl NavicatMain {
    fn prepare_query_parameter_prompt(
        &mut self,
        tab_id: TabId,
        sql: &str,
        execution: PendingQueryExecution,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let specs = query_parameter_specs(sql);
        if specs.is_empty() {
            return false;
        }

        let parameters = specs
            .into_iter()
            .map(|spec| {
                let input =
                    cx.new(|cx| InputState::new(window, cx).placeholder(spec.label.clone()));
                if let Some(value) = self.query_parameter_history.get(&spec.key).cloned() {
                    input.update(cx, |input, cx| input.set_value(value, window, cx));
                }
                QueryParameterInput {
                    key: spec.key,
                    label: spec.label,
                    input,
                }
            })
            .collect::<Vec<_>>();

        let bulk_input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("json")
                .line_number(false)
                .rows(8)
        });

        if let Some(first) = parameters.first() {
            first.input.read(cx).focus_handle(cx).focus(window, cx);
        }

        self.pending_query_parameters = Some(PendingQueryParameterPrompt {
            tab_id,
            sql: sql.to_string(),
            execution,
            parameters,
            active_mode: QueryParameterInputMode::Fields,
            bulk_input,
        });
        cx.notify();
        true
    }

    fn switch_query_parameter_input_mode(
        &mut self,
        mode: QueryParameterInputMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_query_parameters.as_mut() else {
            return;
        };
        pending.active_mode = mode;
        match mode {
            QueryParameterInputMode::Fields => {
                if let Some(first) = pending.parameters.first() {
                    first.input.read(cx).focus_handle(cx).focus(window, cx);
                }
            }
            QueryParameterInputMode::Array => {
                pending.bulk_input.read(cx).focus_handle(cx).focus(window, cx);
            }
        }
        cx.notify();
    }

    fn cancel_query_parameter_prompt(&mut self, cx: &mut Context<Self>) {
        if self.pending_query_parameters.take().is_some() {
            cx.notify();
        }
    }

    fn confirm_query_parameter_prompt(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_query_parameters.take() else {
            return;
        };

        let Some(values) = self.query_parameter_values(&pending, cx) else {
            self.pending_query_parameters = Some(pending);
            cx.notify();
            return;
        };
        let sql = bind_query_parameters(&pending.sql, &values);

        match pending.execution {
            PendingQueryExecution::All { statements } => {
                let statements = self
                    .query_editors
                    .get(&pending.tab_id)
                    .cloned()
                    .map(|editor| {
                        let statements = editor.update(cx, |editor, cx| {
                            // 新编辑器：整篇替换为绑定后的 SQL，并据此重建语句运行列表。
                            let len = editor.text().len();
                            editor.replace_text_range(fluxdb_editor_core::Range::new(0, len), &sql, cx);
                            sql_editor_adapter::build_statement_runs(&sql)
                        });
                        self.dispatch(
                            AppCommand::UpdateQueryText {
                                tab_id: pending.tab_id,
                                text: sql.clone(),
                            },
                            cx,
                        );
                        statements
                    })
                    .unwrap_or(statements);
                if self.request_dangerous_query_confirmation(
                    pending.tab_id,
                    &sql,
                    PendingQueryExecution::All {
                        statements: statements.clone(),
                    },
                    cx,
                ) {
                    return;
                }
                self.start_query_all_execution_resolved(pending.tab_id, statements, Some(sql), cx);
            }
            PendingQueryExecution::Text => {
                if let Some(sql_editor) = self.query_editors.get(&pending.tab_id).cloned() {
                    let replaced = sql_editor.update(cx, |editor, cx| {
                        // 新编辑器：依据选区/全文/光标语句判定参数替换落点并写入。
                        let text = editor.text();
                        let selection = {
                            let r = editor.selection_range();
                            r.start..r.end
                        };
                        match sql_editor_adapter::parameter_replacement_target(
                            &text,
                            selection,
                            &pending.sql,
                        ) {
                            Some(sql_editor_adapter::ParameterReplacementTarget::All) => {
                                let len = text.len();
                                editor
                                    .replace_text_range(fluxdb_editor_core::Range::new(0, len), &sql, cx);
                                true
                            }
                            Some(sql_editor_adapter::ParameterReplacementTarget::Selected(r))
                            | Some(sql_editor_adapter::ParameterReplacementTarget::Statement(r)) => {
                                editor
                                    .replace_text_range(fluxdb_editor_core::Range::new(r.start, r.end), &sql, cx);
                                true
                            }
                            None => false,
                        }
                    });
                    if replaced {
                        let text = sql_editor.read(cx).text();
                        self.dispatch(
                            AppCommand::UpdateQueryText {
                                tab_id: pending.tab_id,
                                text,
                            },
                            cx,
                        );
                    }
                }
                if self.request_dangerous_query_confirmation(
                    pending.tab_id,
                    &sql,
                    PendingQueryExecution::Text,
                    cx,
                ) {
                    return;
                }
                self.start_query_text_execution_resolved(pending.tab_id, sql, cx);
            }
            PendingQueryExecution::Statement { statement } => {
                let Some(sql_editor) = self.query_editors.get(&pending.tab_id).cloned() else {
                    self.show_message("查询编辑器未就绪", AppMessageKind::Warning, cx);
                    cx.notify();
                    return;
                };
                let statement = sql_editor.update(cx, |editor, cx| {
                    // 新编辑器：替换光标所在语句区间为绑定后的 SQL，并重建该语句 run。
                    let text = editor.text();
                    let selection = {
                        let r = editor.selection_range();
                        r.start..r.end
                    };
                    if let Some(target) =
                        sql_editor_adapter::parameter_replacement_target(&text, selection, &statement.text)
                    {
                        match target {
                            sql_editor_adapter::ParameterReplacementTarget::All => {
                                let len = text.len();
                                editor
                                    .replace_text_range(fluxdb_editor_core::Range::new(0, len), &sql, cx);
                            }
                            sql_editor_adapter::ParameterReplacementTarget::Selected(r)
                            | sql_editor_adapter::ParameterReplacementTarget::Statement(r) => {
                                editor
                                    .replace_text_range(fluxdb_editor_core::Range::new(r.start, r.end), &sql, cx);
                            }
                        }
                    }
                    sql_editor_adapter::build_statement_runs(&sql)
                        .into_iter()
                        .next()
                        .unwrap_or(SqlStatementRun {
                            id: SqlStatementId(0),
                            ordinal: 0,
                            start_row: 0,
                            end_row: 0,
                            range: 0..sql.len(),
                            text: sql.clone(),
                        })
                });
                let text = sql_editor.read(cx).text();
                self.dispatch(
                    AppCommand::UpdateQueryText {
                        tab_id: pending.tab_id,
                        text,
                    },
                    cx,
                );
                if self.request_dangerous_query_confirmation(
                    pending.tab_id,
                    &sql,
                    PendingQueryExecution::Statement {
                        statement: statement.clone(),
                    },
                    cx,
                ) {
                    return;
                }
                self.start_query_statement_execution_resolved(
                    pending.tab_id,
                    sql_editor,
                    statement,
                    sql,
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn query_parameter_values(
        &mut self,
        pending: &PendingQueryParameterPrompt,
        cx: &mut Context<Self>,
    ) -> Option<BTreeMap<String, String>> {
        match pending.active_mode {
            QueryParameterInputMode::Fields => Some(self.field_query_parameter_values(pending, cx)),
            QueryParameterInputMode::Array => self.array_query_parameter_values(pending, cx),
        }
    }

    fn field_query_parameter_values(
        &mut self,
        pending: &PendingQueryParameterPrompt,
        cx: &mut Context<Self>,
    ) -> BTreeMap<String, String> {
        let mut values = BTreeMap::new();
        for parameter in &pending.parameters {
            let value = parameter.input.read(cx).value().to_string();
            self.query_parameter_history
                .insert(parameter.key.clone(), value.clone());
            values.insert(parameter.key.clone(), value);
        }
        values
    }

    fn array_query_parameter_values(
        &mut self,
        pending: &PendingQueryParameterPrompt,
        cx: &mut Context<Self>,
    ) -> Option<BTreeMap<String, String>> {
        let text = pending.bulk_input.read(cx).value().to_string();
        let array_values = match parse_query_parameter_array_values(&text) {
            Ok(values) => values,
            Err(message) => {
                self.show_message(message, AppMessageKind::Warning, cx);
                return None;
            }
        };
        if array_values.len() != pending.parameters.len() {
            self.show_message(
                format!(
                    "数组参数数量不匹配，需要 {} 个，当前 {} 个",
                    pending.parameters.len(),
                    array_values.len()
                ),
                AppMessageKind::Warning,
                cx,
            );
            return None;
        }

        let mut values = BTreeMap::new();
        for (parameter, value) in pending.parameters.iter().zip(array_values) {
            self.query_parameter_history
                .insert(parameter.key.clone(), value.clone());
            values.insert(parameter.key.clone(), value);
        }
        Some(values)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QueryParameterSpec {
    key: String,
    label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QueryParameterOccurrence {
    key: String,
    range: Range<usize>,
}

fn query_parameter_specs(sql: &str) -> Vec<QueryParameterSpec> {
    let mut seen = BTreeSet::new();
    query_parameter_occurrences(sql)
        .into_iter()
        .filter_map(|occurrence| {
            if seen.insert(occurrence.key.clone()) {
                Some(QueryParameterSpec {
                    label: query_parameter_label(&occurrence.key),
                    key: occurrence.key,
                })
            } else {
                None
            }
        })
        .collect()
}

fn query_parameter_label(key: &str) -> String {
    key.strip_prefix('?')
        .filter(|index| !index.is_empty() && index.chars().all(|ch| ch.is_ascii_digit()))
        .map(|index| format!("参数 {index}"))
        .unwrap_or_else(|| key.to_string())
}

fn bind_query_parameters(sql: &str, values: &BTreeMap<String, String>) -> String {
    let mut output = sql.to_string();
    let mut occurrences = query_parameter_occurrences(sql);
    occurrences.sort_by(|left, right| right.range.start.cmp(&left.range.start));
    for occurrence in occurrences {
        let Some(value) = values.get(&occurrence.key) else {
            continue;
        };
        output.replace_range(occurrence.range, &sql_parameter_literal(value));
    }
    output
}

fn parse_query_parameter_array_values(text: &str) -> Result<Vec<String>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("请输入参数数组".to_string());
    }

    if trimmed.starts_with('[') {
        let value = serde_json::from_str::<serde_json::Value>(trimmed)
            .map_err(|_| "数组格式无效，请输入如 [1, \"Alice\"]".to_string())?;
        let Some(items) = value.as_array() else {
            return Err("请输入 JSON 数组".to_string());
        };
        return Ok(items.iter().map(query_parameter_json_value_text).collect());
    }

    let values = trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if values.is_empty() {
        Err("请输入参数数组".to_string())
    } else {
        Ok(values)
    }
}

fn query_parameter_json_value_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => value.to_string(),
    }
}

fn query_parameter_occurrences(sql: &str) -> Vec<QueryParameterOccurrence> {
    let bytes = sql.as_bytes();
    let mut occurrences = Vec::new();
    let mut index = 0usize;
    let mut positional = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'\'' | b'"' | b'`' => {
                index = skip_sql_quoted_bytes(bytes, index);
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index = skip_sql_line_comment_bytes(bytes, index + 2);
            }
            b'#' => {
                index = skip_sql_line_comment_bytes(bytes, index + 1);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = skip_sql_block_comment_bytes(bytes, index + 2);
            }
            b'?' => {
                positional += 1;
                occurrences.push(QueryParameterOccurrence {
                    key: format!("?{positional}"),
                    range: index..index + 1,
                });
                index += 1;
            }
            b':' if bytes.get(index + 1).is_some_and(|byte| is_sql_parameter_start(*byte))
                && (index == 0 || bytes.get(index - 1) != Some(&b':')) =>
            {
                let start = index;
                index += 2;
                while bytes
                    .get(index)
                    .is_some_and(|byte| is_sql_parameter_continue(*byte))
                {
                    index += 1;
                }
                let label = &sql[start..index];
                occurrences.push(QueryParameterOccurrence {
                    key: label.to_string(),
                    range: start..index,
                });
            }
            _ => {
                index += 1;
            }
        }
    }

    occurrences
}

fn skip_sql_quoted_bytes(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
            continue;
        }
        if bytes[index] == quote {
            if bytes.get(index + 1) == Some(&quote) {
                index += 2;
                continue;
            }
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

fn skip_sql_line_comment_bytes(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

fn skip_sql_block_comment_bytes(bytes: &[u8], mut index: usize) -> usize {
    while index + 1 < bytes.len() {
        if bytes[index] == b'*' && bytes[index + 1] == b'/' {
            return index + 2;
        }
        index += 1;
    }
    bytes.len()
}

fn is_sql_parameter_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_sql_parameter_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn sql_parameter_literal(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("null") {
        return "NULL".to_string();
    }
    if trimmed.eq_ignore_ascii_case("true") || trimmed.eq_ignore_ascii_case("false") {
        return trimmed.to_ascii_uppercase();
    }
    if is_sql_number_literal(trimmed) {
        return trimmed.to_string();
    }
    if is_quoted_sql_literal(trimmed) {
        return trimmed.to_string();
    }

    format!("'{}'", value.replace('\'', "''"))
}

fn is_sql_number_literal(value: &str) -> bool {
    !value.is_empty()
        && value.chars().any(|ch| ch.is_ascii_digit())
        && value
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, '+' | '-' | '.' | 'e' | 'E'))
        && value.parse::<f64>().is_ok()
}

fn is_quoted_sql_literal(value: &str) -> bool {
    value.len() >= 2
        && ((value.starts_with('\'') && value.ends_with('\''))
            || (value.starts_with('"') && value.ends_with('"')))
}
