/// 失败结果对应的修复请求：结果页序号用于回查本次执行来源。
struct QueryFailureRepair {
    result_index: usize,
    summary: QueryExecutionSummary,
}

impl NavicatMain {
    /// 抽屉发送时自动绑定当前 SQL 选区。
    ///
    /// 只在“当前页是 SQL 编辑器且有非空选区”时成立；用户没有选区时不改写，
    /// 仍按普通问答处理，避免把“解释一下这段 SQL”误判成修改任务。
    /// 写回工具仍校验标签页/目标/文本版本，失败修复路径不改用此自动绑定。
    fn auto_bind_ai_sql_selection(
        &mut self,
        page: &fluxdb_app::CurrentPageContext,
        cx: &Context<Self>,
    ) -> Option<fluxdb_app::SqlEditorEditGrant> {
        if page.kind != fluxdb_app::PageContextKind::SqlEditor
            || self.ai_chat.pending_sql_edit.is_some()
        {
            return None;
        }
        let tab_id = TabId(page.snapshot.tab_id?);
        let tab = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)?;
        let fluxdb_app::TabKind::QueryEditor(query) = &tab.kind else {
            return None;
        };
        if query.running {
            return None;
        }
        let key = QueryPaneKey::new(tab_id, query.active_pane);
        let editor = self.query_editors.get(&key)?;
        let snapshot = editor.read(cx).buffer.snapshot();
        let range = editor.read(cx).selection_range().sorted();
        if range.start >= range.end {
            return None;
        }
        let grant = fluxdb_app::SqlEditorEditGrant {
            tab_id: tab_id.0,
            target: fluxdb_app::TargetRef {
                connection_id: Some(query.connection_id.0 as i64),
                database: query.database.clone(),
                schema: query.schema.clone(),
                ..Default::default()
            },
            snapshot,
            range,
            exclusive_edit: false,
        };
        grant.validate().ok().map(|_| grant)
    }

    /// “用 AI 修复”入口：只把失败 SQL 与错误带进抽屉，不在编辑器侧新增常驻模式。
    fn begin_ai_sql_editor_edit(
        &mut self,
        tab_id: TabId,
        failure: Option<QueryFailureRepair>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ai_chat.running.is_some()
            || self.ai_chat.loading
            || self.ai_chat._io.is_some()
            || self.ai_chat.pending_sql_edit.is_some()
            || self.ai_chat.recovery_pending
            || self.ai_chat.recovery_failed
            || self.ai_chat.attachment_loading
            || self.pending_config_proposal.is_some()
            || self.config_proposal_busy
        {
            self.show_message(tr("app.ai.sql_edit.busy"), AppMessageKind::Warning, cx);
            return;
        }
        if !self.ai_chat.input.read(cx).value().trim().is_empty() {
            self.show_message(
                tr("app.ai.chat.action.unsent_draft"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        let target = match self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
        {
            Some(TabState {
                kind: TabKind::QueryEditor(query),
                ..
            }) if !query.running => fluxdb_app::TargetRef {
                connection_id: Some(query.connection_id.0 as i64),
                database: query.database.clone(),
                schema: query.schema.clone(),
                ..Default::default()
            },
            _ => {
                self.show_message(
                    tr("app.ai.sql_edit.unavailable"),
                    AppMessageKind::Warning,
                    cx,
                );
                return;
            }
        };
        let key = self.query_pane_key(tab_id);
        let Some(editor) = self.query_editors.get(&key) else {
            return;
        };
        let snapshot = editor.read(cx).buffer.snapshot();
        let selected = editor.read(cx).selection_range().sorted();
        let current_statements = self.query_statement_runs(key, cx);
        let mut range_is_selection = false;
        let mut range = if let Some(failure) = failure.as_ref() {
            // 优先按执行时记录的语句身份定位。连接器展示的 SQL 可能已追加 LIMIT、
            // 去掉注释或替换参数，不能再用它和编辑器原文做逐字匹配。
            let source_range = query_result_statement_ordinal_for_tab(
                self.controller.state(),
                key,
                failure.result_index,
            )
            .and_then(|summary_index| {
                query_execution_source_range(
                    self.query_execution_sources
                        .get(&key)
                        .map(Vec::as_slice),
                    summary_index,
                    &current_statements,
                )
            });
            if let Some(range) = source_range {
                fluxdb_editor_core::Range::new(range.start, range.end)
            } else {
                // 兼容没有来源快照的旧结果；仅在完整语句唯一匹配时使用。
                let expected = failure
                    .summary
                    .sql
                    .trim()
                    .trim_end_matches(';')
                    .trim();
                let matches = current_statements
                    .iter()
                    .filter(|statement| {
                        statement.text.trim().trim_end_matches(';').trim() == expected
                    })
                    .collect::<Vec<_>>();
                if matches.len() == 1 {
                    let range = &matches[0].range;
                    fluxdb_editor_core::Range::new(range.start, range.end)
                } else if selected.start < selected.end {
                    range_is_selection = true;
                    selected
                } else {
                    self.show_message(
                        tr("app.ai.sql_edit.locate_failed"),
                        AppMessageKind::Warning,
                        cx,
                    );
                    return;
                }
            }
        } else if selected.start < selected.end {
            range_is_selection = true;
            selected
        } else if let Some(statement) = {
            let cursor = editor.read(cx).cursor_offset().min(snapshot.len());
            current_statements
                .iter()
                .find(|statement| statement.range.start <= cursor && cursor <= statement.range.end)
        } {
            fluxdb_editor_core::Range::new(statement.range.start, statement.range.end)
        } else {
            self.show_message(
                tr("app.ai.sql_edit.locate_failed"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        };
        // 语句切分范围不含分号；整条语句改写包含其分隔符，避免替换后留下双分号。
        // 显式选区则严格保留用户指定的边界。
        if !range_is_selection
            && range.end < snapshot.len()
            && snapshot.text_in_range(fluxdb_editor_core::Range::new(range.end, range.end + 1))
                == ";"
        {
            range.end += 1;
        }
        let grant = fluxdb_app::SqlEditorEditGrant {
            tab_id: tab_id.0,
            target,
            snapshot,
            range,
            exclusive_edit: true,
        };
        if grant.validate().is_err() {
            self.show_message(tr("app.ai.sql_edit.invalid"), AppMessageKind::Warning, cx);
            return;
        }
        self.ai_chat.pending_sql_edit = Some(grant);
        if let Some(failure) = failure {
            let prompt = tr("app.ai.sql_edit.repair_prompt").replace(
                "%{error}",
                &failure
                    .summary
                    .message
                    .chars()
                    .take(4096)
                    .collect::<String>(),
            );
            if self.ai_chat.loaded {
                self.ai_chat
                    .input
                    .update(cx, |input, cx| input.set_value(prompt, window, cx));
                self.open_ai_chat(window, cx);
                self.send_ai_chat(window, cx);
            } else {
                self.ai_chat.pending_fix_prompt = Some(prompt);
                self.open_ai_chat(window, cx);
            }
        } else {
            self.open_ai_chat(window, cx);
        }
    }

    fn apply_ai_sql_editor_edit(
        &mut self,
        request: std::sync::Arc<fluxdb_app::SqlEditorEditRequest>,
        cx: &mut Context<Self>,
    ) {
        let tab_id = TabId(request.grant.tab_id);
        let target = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::QueryEditor(query) if !query.running => Some(fluxdb_app::TargetRef {
                    connection_id: Some(query.connection_id.0 as i64),
                    database: query.database.clone(),
                    schema: query.schema.clone(),
                    ..Default::default()
                }),
                _ => None,
            });
        let key = self.query_pane_key(tab_id);
        let result = match (target, self.query_editors.get(&key).cloned()) {
            (Some(target), Some(editor)) => editor.update(cx, |editor, cx| {
                request.apply(&editor.buffer.snapshot(), &target, |range, sql| {
                    // 复用一次编辑事务与 Changed → AppCommand 增量同步，不保存也不运行。
                    editor.replace_text_range(range, sql, cx);
                })
            }),
            _ => {
                request.reject("sql_edit_unavailable");
                Err("sql_edit_unavailable")
            }
        };
        match result {
            Ok(()) => self.show_message(tr("app.ai.sql_edit.applied"), AppMessageKind::Success, cx),
            Err(code) => self.show_message(
                localize_backend_error_text(code),
                AppMessageKind::Warning,
                cx,
            ),
        }
    }
}

/// 按本次执行的语句快照定位当前编辑器范围。
///
/// `sources` 与执行结果摘要同序：整篇执行时为语句顺序，单条执行为单元素。
/// 语句 id 含序号和原文，因此重复 SQL 也能区分；执行后若目标语句内容或序号变化，
/// 返回 `None` 而不是猜测位置。
fn query_execution_source_range(
    sources: Option<&[SqlStatementRun]>,
    summary_index: usize,
    current_statements: &[SqlStatementRun],
) -> Option<std::ops::Range<usize>> {
    let source = sources?.get(summary_index)?;
    current_statements
        .iter()
        .find(|statement| statement.id == source.id && statement.text == source.text)
        .map(|statement| statement.range.clone())
}

#[cfg(test)]
mod query_execution_source_tests {
    use super::*;

    #[test]
    fn repeated_sql_uses_the_executed_summary_index() {
        let statements =
            sql_editor_adapter::build_statement_runs(
                "SELECT 1;\nSELECT 1;\nSELECT 3;",
                fluxdb_core::DatabaseKind::MySql,
            );
        let range = query_execution_source_range(Some(&statements), 1, &statements)
            .expect("second repeated statement");

        assert_eq!(range, statements[1].range);
        assert_ne!(range, statements[0].range);
    }

    #[test]
    fn single_statement_execution_maps_to_its_editor_statement() {
        let statements = sql_editor_adapter::build_statement_runs(
            "SELECT 'first';\nSELEC 1;\nSELECT 3;",
            fluxdb_core::DatabaseKind::MySql,
        );
        let sources = vec![statements[1].clone()];
        let range = query_execution_source_range(Some(&sources), 0, &statements)
            .expect("failed statement");

        assert_eq!(range, statements[1].range);
    }

    #[test]
    fn edited_source_is_not_reused_for_a_different_statement() {
        let sources =
            sql_editor_adapter::build_statement_runs(
                "SELECT 1;\nSELEC 1;",
                fluxdb_core::DatabaseKind::MySql,
            );
        let current =
            sql_editor_adapter::build_statement_runs(
                "SELECT 1;\nSELECT 3;",
                fluxdb_core::DatabaseKind::MySql,
            );

        assert_eq!(
            query_execution_source_range(Some(&sources), 1, &current),
            None
        );
    }
}

// 无头测试在模型服务边界提供固定 SQL；仍运行生产工具、动作回桥和编辑器事件。
#[cfg(test)]
fn run_sql_editor_edit_fixture(
    sql: String,
    grant: Option<fluxdb_app::EditorGrant>,
    job: fluxdb_app::AiConversationJob,
    actions: std::sync::Arc<dyn fluxdb_app::UiActionSink>,
    cancel: &fluxdb_core::CancelFlag,
    emit: &mut dyn FnMut(&fluxdb_core::ModelEvent),
) -> fluxdb_core::Result<Vec<fluxdb_core::ModelEvent>> {
    let grant = grant.and_then(|grant| grant.sql_edit).ok_or_else(|| {
        fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal, "sql_edit_unavailable")
    })?;
    let mut tools = fluxdb_app::ToolCatalog::new(Vec::new());
    fluxdb_app::add_ai_sql_editor_edit_tool(&mut tools, grant, actions)?;
    let args = serde_json::json!({"sql":sql}).to_string();
    tools
        .prepare("editor.apply_edit", &args)
        .expect("fixture schema");
    let receipt = fluxdb_app::ai_executor::block_on_cancellable(
        tools
            .executable("editor.apply_edit")
            .expect("production edit tool")
            .execute(
                fluxdb_core::ToolExecutionContext {
                    session_id: job.session_id,
                    turn_id: job.turn_id,
                    iteration: 1,
                    call_id: "sql-edit-fixture".into(),
                    cancel: cancel.clone(),
                    automatic_approval: false,
                    bounded_write: false,
                    progress: None,
                },
                args,
            ),
        cancel,
    )??;
    if !receipt.is_success() {
        return Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Internal,
            receipt
                .error_code
                .unwrap_or_else(|| "sql_edit_unavailable".into()),
        ));
    }
    let event = fluxdb_core::ModelEvent::Finished(fluxdb_core::FinishReason::Stop);
    emit(&event);
    Ok(vec![event])
}
