impl NavicatMain {
    /// 当前查询标签的聚焦屏；标签不存在或不是查询编辑器时返回 None。
    fn active_query_pane_key(&self, tab_id: TabId) -> Option<QueryPaneKey> {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::QueryEditor(editor) => {
                    Some(QueryPaneKey::new(tab_id, editor.active_pane))
                }
                _ => None,
            })
    }

    /// 聚焦屏键的兜底版本：调用方已经确定是查询标签页。
    fn query_pane_key(&self, tab_id: TabId) -> QueryPaneKey {
        self.active_query_pane_key(tab_id)
            .unwrap_or(QueryPaneKey::new(tab_id, QueryPaneId(1)))
    }

    /// 依据宿主当前明暗模式 + 用户所选主题名返回通用编辑器的主题配色。
    ///
    /// 从主题注册表解析所选主题（settings.dark_theme/light_theme）的
    /// highlight/colors 派生，缺字段回退到内置默认两套（table_state/theme_registry）。
    fn editor_theme_for(&self, cx: &App) -> editor_component::EditorTheme {
        let theme_name = if self.theme_mode == ThemeMode::Dark {
            self.settings_editor_draft.dark_theme.as_str()
        } else {
            self.settings_editor_draft.light_theme.as_str()
        };
        crate::editor_theme_for(self.theme_mode, theme_name, cx)
    }

    fn redis_data_table_width(&self, tab_id: TabId, window: &Window) -> Option<Pixels> {
        self.is_redis_data_tab(tab_id).then(|| {
            let viewport = f32::from(window.viewport_size().width);
            let side_width = if self.show_connection_browser {
                clamp_connection_browser_width(self.connection_browser_width)
            } else {
                28.
            };
            px((viewport - side_width - 8.).max(360.).min(viewport))
        })
    }

    fn sync_table_hover_overlay_block(&mut self, cx: &mut Context<Self>) {
        let should_block = self.data_filter_popover.is_some()
            || self.field_filter_popover.is_some()
            || self.local_filter_popover.is_some()
            || self.local_filter_manager_popover.is_some();
        if should_block == self.table_hover_blocked_by_overlay {
            return;
        }

        self.table_hover_blocked_by_overlay = should_block;
        if should_block {
            self.set_table_hover_transparent(cx);
        } else {
            self.restore_table_hover_if_ready(cx);
        }
    }

    fn set_table_hover_transparent(&mut self, cx: &mut Context<Self>) {
        if self.table_hover_color.is_none() {
            self.table_hover_color = Some(cx.theme().table_hover);
        }
        ComponentTheme::global_mut(cx).colors.table_hover = cx.theme().transparent;
    }

    fn restore_table_hover_if_ready(&mut self, cx: &mut Context<Self>) {
        if self.table_hover_blocked_by_overlay {
            return;
        }
        self.table_hover_color = None;
        ComponentTheme::global_mut(cx).colors.table_hover = cx.theme().table_hover;
    }

    fn data_table_state(
        &mut self,
        tab_id: TabId,
        page: &DataPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TableState<DataPageTableDelegate>> {
        if let Some(table_state) = self.data_table_states.get(&tab_id) {
            return table_state.clone();
        }

        let display_page = self.data_page_for_display(tab_id, page);
        let page = match display_page {
            Cow::Borrowed(page) => Arc::new(page.clone()),
            Cow::Owned(page) => Arc::new(page),
        };
        let changes = self.data_changes_for_tab(tab_id);
        let column_choices = self.column_choices_for_tab(tab_id);
        // 这里只在首次创建 table_state 时做 Redis 展示过滤，render 命中已有状态时不再重复算。
        let mut delegate = DataPageTableDelegate::from_page_with_rule(
            cx.entity().downgrade(),
            tab_id,
            DATA_EDITOR_PANE,
            page,
            self.data_sort_rules.get(&tab_id).map(Vec::as_slice),
            self.visible_table_fields.get(&tab_id),
            &column_choices,
            changes.as_ref(),
            self.data_search_queries.get(&tab_id).map(String::as_str),
            self.data_search_active_matches.get(&tab_id).copied(),
            self.data_search_highlight_all_tabs.contains(&tab_id),
            self.table_info_highlighted_column(tab_id),
            None,
            self.data_cell_edit_input.clone(),
            self.data_cell_editing.clone(),
            self.temporal_part_input.clone(),
            self.temporal_part_editing,
            true,
            self.redis_data_table_width(tab_id, window),
            self.data_table_sort_handler(cx),
        );
        self.apply_foreign_key_targets_to_delegate(tab_id, &mut delegate);
        if !delegate.redis_page {
            self.apply_data_table_column_widths(&mut delegate);
        }
        let table_state = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .sortable(false)
                .col_movable(false)
                .col_resizable(true)
                .col_selectable(false)
        });
        self.subscribe_data_table_widths(
            QueryPaneKey::new(tab_id, DATA_EDITOR_PANE),
            &table_state,
            cx,
        );
        self.data_table_states.insert(tab_id, table_state.clone());
        table_state
    }

    fn query_result_table_state_from_sorted_page(
        &mut self,
        key: QueryPaneKey,
        page: Arc<DataPage>,
        page_index: usize,
        sort_rules: &[DataSortRule],
        source_row_indexes: &[usize],
        result_editor: Option<&DataEditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TableState<DataPageTableDelegate>> {
        let tab_id = key.tab_id;
        let changes = result_editor.and_then(|editor| editor.changes.as_ref());
        let editing_cell = self
            .data_cell_editing
            .filter(|editing| editing.tab_id == tab_id && editing.pane_id == key.pane_id)
            .filter(|editing| editing.query_result_page_index == Some(page_index));
        let cells_editable = query_result_cells_editable(result_editor.is_some(), sort_rules);
        let column_choices = BTreeMap::new();
        let mut delegate = DataPageTableDelegate::from_page_with_rule(
            cx.entity().downgrade(),
            tab_id,
            key.pane_id,
            page,
            Some(sort_rules),
            None,
            &column_choices,
            changes,
            self.data_search_queries.get(&tab_id).map(String::as_str),
            self.data_search_active_matches.get(&tab_id).copied(),
            self.data_search_highlight_all_tabs.contains(&tab_id),
            None,
            Some(page_index),
            self.data_cell_edit_input.clone(),
            editing_cell,
            self.temporal_part_input.clone(),
            self.temporal_part_editing.filter(|editing| {
                matches!(
                    editing.target,
                    TemporalEditTarget::DataCell(cell)
                        if cell.tab_id == tab_id
                            && cell.query_result_page_index == Some(page_index)
                )
            }),
            cells_editable,
            None,
            self.query_result_sort_handler(cx),
        );
        apply_data_table_source_row_indexes(&mut delegate, source_row_indexes);
        self.apply_data_table_column_widths(&mut delegate);
        if let Some(table_state) = self.query_result_table_states.get(&key) {
            table_state.update(cx, |table, cx| {
                let old = table.delegate();
                let columns_changed = !data_table_columns_match(&old.columns, &delegate.columns);
                let content_changed = data_table_rows_or_sorts_changed(
                    &old.page,
                    &old.sorts,
                    &delegate.page,
                    &delegate.sorts,
                );
                let search_changed = old.search_matches != delegate.search_matches
                    || old.active_search_match != delegate.active_search_match
                    || old.highlight_search_matches != delegate.highlight_search_matches;
                let selected_cell = old.selected_cell;
                let selected_row = old.selected_row;
                let selected_cells = old.selected_cells.clone();
                let selected_rows = old.selected_rows.clone();
                let selection_anchor = old.selection_anchor;
                let hovered_cell = old.hovered_cell;
                let mut delegate = delegate;
                delegate.selected_cell = selected_cell;
                delegate.selected_row = selected_row;
                delegate.selected_cells = selected_cells;
                delegate.selected_rows = selected_rows;
                delegate.selection_anchor = selection_anchor;
                delegate.hovered_cell = hovered_cell;
                *table.delegate_mut() = delegate;
                if columns_changed || content_changed || search_changed {
                    table.refresh(cx);
                } else {
                    cx.notify();
                }
            });
            return table_state.clone();
        }

        let table_state = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .sortable(false)
                .col_movable(false)
                .col_resizable(true)
                .col_selectable(false)
        });
        self.subscribe_data_table_widths(key, &table_state, cx);
        self.query_result_table_states
            .insert(key, table_state.clone());
        table_state
    }

    fn refresh_query_result_table_state(
        &mut self,
        key: QueryPaneKey,
        page: &DataPage,
        page_index: usize,
        result_index: usize,
        result_editor: Option<&DataEditorState>,
        cx: &mut Context<Self>,
    ) -> bool {
        let tab_id = key.tab_id;
        let Some(table_state) = self.query_result_table_states.get(&key).cloned() else {
            return false;
        };
        let sort_rules = self
            .query_result_sort_rules
            .get(&QueryResultSortKey {
                tab_id,
                pane_id: key.pane_id,
                result_index,
            })
            .cloned()
            .unwrap_or_default();
        let sorted_page =
            self.query_result_display_page(key, result_index, page_index, page, &sort_rules, true);
        let changes = result_editor.and_then(|editor| editor.changes.as_ref());
        let editing_cell = self
            .data_cell_editing
            .filter(|editing| editing.tab_id == tab_id && editing.pane_id == key.pane_id)
            .filter(|editing| editing.query_result_page_index == Some(page_index));
        let cells_editable = query_result_cells_editable(result_editor.is_some(), &sort_rules);
        let column_choices = BTreeMap::new();
        let mut delegate = DataPageTableDelegate::from_page_with_rule(
            cx.entity().downgrade(),
            tab_id,
            key.pane_id,
            sorted_page.page.clone(),
            Some(&sort_rules),
            None,
            &column_choices,
            changes,
            self.data_search_queries.get(&tab_id).map(String::as_str),
            self.data_search_active_matches.get(&tab_id).copied(),
            self.data_search_highlight_all_tabs.contains(&tab_id),
            None,
            Some(page_index),
            self.data_cell_edit_input.clone(),
            editing_cell,
            self.temporal_part_input.clone(),
            self.temporal_part_editing.filter(|editing| {
                matches!(
                    editing.target,
                    TemporalEditTarget::DataCell(cell)
                        if cell.tab_id == tab_id
                            && cell.query_result_page_index == Some(page_index)
                )
            }),
            cells_editable,
            None,
            self.query_result_sort_handler(cx),
        );
        apply_data_table_source_row_indexes(&mut delegate, &sorted_page.source_row_indexes);
        self.apply_data_table_column_widths(&mut delegate);
        table_state.update(cx, |table, cx| {
            let old = table.delegate();
            let columns_changed = !data_table_columns_match(&old.columns, &delegate.columns);
            let content_changed = data_table_rows_or_sorts_changed(
                &old.page,
                &old.sorts,
                &delegate.page,
                &delegate.sorts,
            );
            let search_changed = old.search_matches != delegate.search_matches
                || old.active_search_match != delegate.active_search_match
                || old.highlight_search_matches != delegate.highlight_search_matches;
            let selected_cell = old.selected_cell;
            let selected_row = old.selected_row;
            let selected_cells = old.selected_cells.clone();
            let selected_rows = old.selected_rows.clone();
            let selection_anchor = old.selection_anchor;
            let hovered_cell = old.hovered_cell;
            delegate.selected_cell = selected_cell;
            delegate.selected_row = selected_row;
            delegate.selected_cells = selected_cells;
            delegate.selected_rows = selected_rows;
            delegate.selection_anchor = selection_anchor;
            delegate.hovered_cell = hovered_cell;
            *table.delegate_mut() = delegate;
            if columns_changed || content_changed || search_changed {
                table.refresh(cx);
            } else {
                cx.notify();
            }
        });
        true
    }

    fn subscribe_data_table_widths(
        &mut self,
        key: QueryPaneKey,
        table_state: &Entity<TableState<DataPageTableDelegate>>,
        cx: &mut Context<Self>,
    ) {
        let subscription =
            cx.subscribe(table_state, |this: &mut NavicatMain, table, event: &TableEvent, cx| {
                let TableEvent::ColumnWidthsChanged(widths) = event else {
                    return;
                };
                table.update(cx, |table, _| {
                    apply_data_table_column_widths_from_list(
                        &mut table.delegate_mut().columns,
                        widths,
                    );
                });
                let delegate = table.read(cx).delegate();
                let key = DataTableWidthKey {
                    tab_id: delegate.tab_id,
                    pane_id: delegate.pane_id,
                    query_result_page_index: delegate.query_result_page_index,
                };
                let widths = delegate
                    .columns
                    .iter()
                    .zip(widths.iter())
                    .map(|(column, width)| (column.key.to_string(), *width))
                    .collect::<BTreeMap<_, _>>();
                this.data_table_column_widths.insert(key, widths);
            });
        // 查询结果按屏保存订阅；数据页仍按标签保存。
        if key.pane_id == DATA_EDITOR_PANE {
            self._data_table_width_subscriptions
                .insert(key.tab_id, subscription);
        } else {
            self._query_result_table_width_subscriptions
                .insert(key, subscription);
        }
    }

    fn apply_data_table_column_widths(&self, delegate: &mut DataPageTableDelegate) {
        let key = DataTableWidthKey {
            tab_id: delegate.tab_id,
            pane_id: delegate.pane_id,
            query_result_page_index: delegate.query_result_page_index,
        };
        let Some(widths) = self.data_table_column_widths.get(&key) else {
            return;
        };
        apply_data_table_column_widths(&mut delegate.columns, widths);
    }

    fn query_result_sort_handler(&self, cx: &mut Context<Self>) -> DataTableSortHandler {
        let view = cx.entity().downgrade();
        Arc::new(move |pane_key, field, direction, cx| {
            let _ = view.update(cx, |this, cx| {
                let result_index = this
                    .query_output_tabs
                    .get(&pane_key)
                    .and_then(|tab| tab.result_index())
                    .unwrap_or(0);
                let key = QueryResultSortKey {
                    tab_id: pane_key.tab_id,
                    pane_id: pane_key.pane_id,
                    result_index,
                };
                apply_query_result_header_sort(
                    &mut this.query_result_sort_rules,
                    key,
                    field,
                    direction,
                );
                this.clear_query_result_display_pages_for_result(pane_key, result_index);
                cx.notify();
            });
        })
    }

    fn clear_query_result_display_pages_for_tab(&mut self, tab_id: TabId) {
        self.query_result_display_pages
            .retain(|key, _| key.tab_id != tab_id);
    }

    fn clear_query_result_display_pages_for_result(
        &mut self,
        pane_key: QueryPaneKey,
        result_index: usize,
    ) {
        self.query_result_display_pages.retain(|key, _| {
            key.tab_id != pane_key.tab_id
                || key.pane_id != pane_key.pane_id
                || key.result_index != result_index
        });
    }

    fn query_result_display_page(
        &mut self,
        pane_key: QueryPaneKey,
        result_index: usize,
        page_index: usize,
        page: &DataPage,
        sort_rules: &[DataSortRule],
        refresh_cache: bool,
    ) -> SortedQueryResultPage {
        let key = QueryResultDisplayKey {
            tab_id: pane_key.tab_id,
            pane_id: pane_key.pane_id,
            result_index,
            page_index,
        };
        if !refresh_cache
            && let Some(display_page) = self.query_result_display_pages.get(&key)
        {
            return display_page.clone();
        }

        let display_page = sorted_query_result_page(page, sort_rules);
        self.query_result_display_pages
            .insert(key, display_page.clone());
        display_page
    }

    fn query_editor_state(
        &mut self,
        key: QueryPaneKey,
        editor: &QueryEditorState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<editor_component::Editor> {
        let Some(pane) = editor.pane(key.pane_id) else {
            // 屏已被关闭：返回一个临时编辑器只为渲染兜底，不写入缓存。
            return cx.new(|cx| {
                editor_component::Editor::new(
                    String::new(),
                    editor_component::Providers::default(),
                    None,
                    window,
                    cx,
                )
            });
        };
        if let Some(sql_editor) = self.query_editors.get(&key) {
            let sql_editor = sql_editor.clone();
            let settings = self.controller.state().settings.clone();
            sql_editor.update(cx, |sql_editor, cx| {
                apply_code_editor_settings(sql_editor, &settings, cx);
                sql_editor.sync_text_silent(&pane.text, cx);
            });
            return sql_editor;
        }

        let initial_text = pane.text.clone();
        let settings = self.controller.state().settings.clone();
        // 构造 SQL 接入层：按连接类型选择方言，注入通用编辑器的补全 / 语法 / 执行通道。
        // 切分用的数据库类型单独保留：Oracle / 达梦在 `SqlDialect` 里都归成 PostgreSQL，
        // 只靠方言会丢掉 `/` 分隔符与 PL/SQL 块边界，编辑器就会把匿名块按 `;` 切碎。
        let database_kind = self.connection_database_kind(editor.connection_id);
        let dialect = sql_editor_adapter::SqlDialect::from_database_kind(database_kind);
        // 语句执行状态仓库：宿主与适配器共享，宿主写状态、适配器还原为行背景装饰。
        let status_store: sql_editor_adapter::SqlStatusStore =
            std::sync::Arc::new(std::sync::Mutex::new(sql_editor_adapter::SqlStatementStatusMap::default()));
        self.query_statement_statuses.insert(key, status_store.clone());
        // 注入「已加载到内存」的 schema：表 / 列补全来源。只读已加载状态，无任何数据库调用。
        let schema = loaded_schema_context(
            self.controller.state(),
            editor.connection_id,
            editor.database.as_deref(),
        );
        // 所有 SQL 补全都复用 fluxdb-app 已有的上下文/索引能力；adapter 仅在 resolver
        // 不可用时回退到本地候选。
        let completion_controller = self.controller.clone();
        let completion_connection_id = editor.connection_id;
        let completion_database = editor.database.clone();
        let completion_schema = editor.schema.clone();
        // F004：采纳补全时回写个性化（recency/frequency）。与补全解析同一 controller。
        let accept_controller = completion_controller.clone();
        // 函数签名提示：`COUNT(|` 给出参数骨架。只查连接方言的静态内置表，不发数据库请求。
        let signature_controller = completion_controller.clone();
        let signature_connection_id = editor.connection_id;
        // F005：metadata 详情解析复用同一 controller：优先读内存 CompletionIndex，索引未覆盖
        // 该对象时按 (库, schema, 表) 取一次列元数据并写回索引。
        let documentation_controller = completion_controller.clone();
        let completion_resolver: sql_editor_adapter::SqlCompletionResolver =
            std::sync::Arc::new(move |text, cursor, explicit, latest_request, request_id| {
                let result = completion_controller
                    .query_completions_for_text_with_cancel(
                        completion_connection_id,
                        completion_database.clone(),
                        completion_schema.clone(),
                        text,
                        cursor,
                        explicit,
                        latest_request,
                        request_id,
                    )
                    .map_err(|error| error.to_string())?;
                let replace_range = fluxdb_editor_core::Range::new(result.replace_start, result.replace_end);
                let items = result
                    .items
                    .into_iter()
                    .map(|item| {
                        let kind = match item.kind {
                            fluxdb_core::QueryCompletionKind::Keyword => fluxdb_editor_core::CompletionKind::Keyword,
                            fluxdb_core::QueryCompletionKind::Function => fluxdb_editor_core::CompletionKind::Function,
                            fluxdb_core::QueryCompletionKind::Table | fluxdb_core::QueryCompletionKind::View => fluxdb_editor_core::CompletionKind::Table,
                            fluxdb_core::QueryCompletionKind::Column => fluxdb_editor_core::CompletionKind::Column,
                            fluxdb_core::QueryCompletionKind::Schema => fluxdb_editor_core::CompletionKind::Schema,
                            fluxdb_core::QueryCompletionKind::Procedure | fluxdb_core::QueryCompletionKind::Trigger => fluxdb_editor_core::CompletionKind::Method,
                            _ => fluxdb_editor_core::CompletionKind::Text,
                        };
                        let mut completion = fluxdb_editor_core::CompletionItem::new(&item.label, kind);
                        completion.insert_text = item.insert_text;
                        completion.detail = item.detail.unwrap_or_default();
                        completion.documentation = item.documentation.unwrap_or_default();
                        completion.filter_text = item.filter_text.unwrap_or_default();
                        completion.sort_text = item.sort_text.unwrap_or_default();
                        completion.replace_range = Some(replace_range);
                        // 候选身份（schema 作用域）随候选带到详情请求，供按完整身份查列。
                        completion.schema = item.schema;
                        completion
                    })
                    .collect();
                Ok(sql_editor_adapter::SqlCompletionResponse { items, has_more: false })
            });
        // F005：补全候选项右侧 metadata 详情。优先复用 fluxdb-app 内存 CompletionIndex；
        // 索引未覆盖该对象（如 search_path 之外的 schema 表）时按 (库, schema, 表) 取一次
        // 元数据并写回索引。latest_request/request_id 传入 AppController，既能 latest-wins
        // 丢弃旧请求，也让被切走的旧请求尽早停止远程查询。
        let documentation_connection_id = editor.connection_id;
        let documentation_database = editor.database.clone();
        let documentation_resolver: sql_editor_adapter::SqlDocumentationResolver =
            std::sync::Arc::new(move |kind, label, comment, schema, latest_request, request_id| {
                use sql_editor_adapter::SqlDocState as S;
                let qkind = match kind {
                    fluxdb_editor_core::CompletionKind::Keyword => fluxdb_core::QueryCompletionKind::Keyword,
                    fluxdb_editor_core::CompletionKind::Function => fluxdb_core::QueryCompletionKind::Function,
                    fluxdb_editor_core::CompletionKind::Table => fluxdb_core::QueryCompletionKind::Table,
                    fluxdb_editor_core::CompletionKind::Column => fluxdb_core::QueryCompletionKind::Column,
                    fluxdb_editor_core::CompletionKind::Schema => fluxdb_core::QueryCompletionKind::Schema,
                    fluxdb_editor_core::CompletionKind::Method => fluxdb_core::QueryCompletionKind::Procedure,
                    _ => fluxdb_core::QueryCompletionKind::Keyword,
                };
                // 编辑器侧已有令牌 + 选中项守卫（旧结果不上屏）；这里再把请求 id 传下去，
                // 让详情解析在切换选中项后提前收敛，避免旧请求继续占用连接做无谓查询。
                let should_cancel = || {
                    latest_request.load(std::sync::atomic::Ordering::Acquire) != request_id
                };
                match documentation_controller.completion_documentation_for_with_cancel(
                    documentation_connection_id,
                    documentation_database.clone(),
                    qkind,
                    label,
                    comment,
                    schema,
                    &should_cancel,
                ) {
                    fluxdb_app::CompletionDocumentationState::Loading => S::Loading,
                    fluxdb_app::CompletionDocumentationState::Ready(text) => S::Ready(text),
                    fluxdb_app::CompletionDocumentationState::Error(reason) => S::Error(reason),
                }
            });
        let signature_resolver: sql_editor_adapter::SqlSignatureResolver =
            std::sync::Arc::new(move |name: &str| {
                signature_controller.sql_function_signature(signature_connection_id, name)
            });
        let adapter = std::sync::Arc::new(
            sql_editor_adapter::SqlAdapter::new(dialect, database_kind)
                .with_status_store(status_store)
                .with_schema(schema)
                .with_completion_resolver(completion_resolver)
                .with_documentation_resolver(documentation_resolver)
                .with_signature_resolver(signature_resolver),
        );
        let language_registry = std::sync::Arc::new(fluxdb_editor_core::LanguageRegistry::new());
        language_registry.register(
            dialect.language_id(),
            std::sync::Arc::new(sql_editor_adapter::SqlLanguage::new(dialect, database_kind)),
            Some(adapter.clone() as _),
        );
        let providers = editor_component::Providers {
            language_registry: Some(language_registry),
            // 语法诊断在后台按版本计算；语义诊断由 metadata provider 逐步补齐，
            // metadata 未加载时不阻塞输入，也不伪造错误。
            diagnostics: Some(adapter.clone() as _),
            completion: Some(adapter.clone() as _),
            execution: Some(adapter.clone() as _),
            decorations: Some(adapter.clone() as _),
            code_lens: Some(adapter.clone() as _),
            signature: Some(adapter.clone() as _),
            ..Default::default()
        };
        let mut profile = fluxdb_editor_core::EditorProfile::default();
        profile.language_id = dialect.language_id().to_string();
        profile.show_line_numbers = true;
        profile.show_folding = true;
        profile.completion_trigger = fluxdb_editor_core::CompletionTrigger::Auto;
        profile.completion_trigger_chars = vec!['.', ' '];
        // 前缀≥2 才自动触发补全：单字符（含常见不停顿输入）不再触发 SQL schema 扫描，
        // 避免大文档每次按键都拉起 completion provider（性能诊断：comp 任务挂起会压帧）。
        // 注：`.` 与 ` ` 走 trigger_chars 独立分支，不受本前缀门控，`SELECT u.` 仍即时触发。
        profile.completion_min_prefix = 2;
        // Tab 宽度接入「设置→编辑器」的 Tab 宽度取值（2/4/8）。
        profile.tab_size = settings.editor_tab_width.max(1) as usize;
        let config = fluxdb_editor_core::EditorConfig {
            profile,
            font: code_font_family(&settings),
            font_size: settings.editor_font_size.clamp(10, 24) as f32,
            line_height: settings.editor_line_height.clamp(13, 28) as f32,
            gutter_line_numbers: true,
        };
        let sql_editor = cx.new(|cx| {
            editor_component::Editor::new(initial_text, providers, Some(config), window, cx)
        });
        adapter.set_editor_id(sql_editor.read(cx).perf_editor_id());
        // 按宿主当前主题名注入编辑器配色（theme_registry 派生，缺字段回退内置默认）。
        let editor_theme = self.editor_theme_for(cx);
        sql_editor.update(cx, |editor, ecx| editor.set_theme(editor_theme, ecx));
        let subscription = cx.subscribe_in(
            &sql_editor,
            window,
            move |this: &mut NavicatMain, editor, event, window, cx| match event {
                editor_component::EditorEvent::Changed(change) => {
                    // 增量同步：按 TextChange 就地更新查询模型文本，避免逐按键全文读取。
                    this.apply_query_incremental_change(key, editor.clone(), change, cx);
                }
                editor_component::EditorEvent::Execute { range, mode } => {
                    this.start_query_execute_event(
                        key,
                        editor.clone(),
                        range.clone(),
                        *mode,
                        window,
                        cx,
                    );
                }
                editor_component::EditorEvent::CodeLensActivated { range, action } => {
                    match action.as_str() {
                        "sql.run" => this.start_query_execute_event(
                            key,
                            editor.clone(),
                            range.clone(),
                            fluxdb_editor_core::ExecuteMode::Execute,
                            window,
                            cx,
                        ),
                        "sql.select" => editor.update(cx, |editor, cx| {
                            editor.select_range(*range, cx);
                        }),
                        _ => tracing::warn!(target: "gdb_editor", action = %action, "unknown CodeLens action"),
                    }
                }
                editor_component::EditorEvent::CompletionAccepted(item) => {
                    // F004：采纳补全 → 回写匿名「采纳历史」。只记 label，不记完整 SQL /
                    // 敏感值；关闭个性化时由 fluxdb-app 内部直接忽略（不记录、不产生排序偏移）。
                    accept_controller.record_completion_accept(&item.label);
                }
                _ => {}
            },
        );
        self.query_editors.insert(key, sql_editor.clone());
        self._query_editor_subscriptions.insert(key, subscription);
        sql_editor
    }

    /// 把模型文本同步进编辑器实体。传 `None` 时同步该标签的所有屏。
    fn sync_query_editor_text(&mut self, tab_id: TabId, pane_id: Option<QueryPaneId>, cx: &mut Context<Self>) {
        let Some(tab) = self.controller.state().tabs.iter().find(|tab| tab.id == tab_id)
        else {
            return;
        };
        let TabKind::QueryEditor(editor) = &tab.kind else {
            return;
        };
        let panes: Vec<(QueryPaneId, String)> = editor
            .panes
            .iter()
            .filter(|pane| pane_id.is_none_or(|id| id == pane.id))
            .map(|pane| (pane.id, pane.text.clone()))
            .collect();
        for (id, text) in panes {
            if let Some(sql_editor) = self
                .query_editors
                .get(&QueryPaneKey::new(tab_id, id))
                .cloned()
            {
                sql_editor.update(cx, |sql_editor, cx| {
                    sql_editor.sync_text_silent(&text, cx);
                });
            }
        }
    }

    /// 按增量 TextChange 就地更新查询模型文本，并仅在内容变化时回写。
    ///
    /// 替代旧实现里 `EditorEvent::Changed` → 逐按键 `Editor::text()` 全文同步：
    /// 这里用宿主已持有的模型文本 + `old_range/new_text` 就地 `replace_range`，
    /// 避免对编辑器实体做跨实体全文读取与整文本拷贝。
    fn apply_query_incremental_change(
        &mut self,
        key: QueryPaneKey,
        _editor: Entity<editor_component::Editor>,
        change: &fluxdb_editor_core::TextChange,
        cx: &mut Context<Self>,
    ) {
        let tab_id = key.tab_id;
        let Some(pane_text) = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::QueryEditor(editor) => editor.pane(key.pane_id).map(|pane| pane.text.clone()),
                _ => None,
            })
        else {
            return;
        };
        if change.full_document {
            if pane_text != change.new_text {
                self.dispatch(
                    AppCommand::UpdateQueryText {
                        tab_id,
                        pane_id: key.pane_id,
                        text: change.new_text.clone(),
                    },
                    cx,
                );
            }
            return;
        }
        let mut updated = pane_text;
        // 内核保证 old_range 落在字符边界；此处与模型文本对齐后就地替换。
        let lo = change.old_range.start.min(updated.len());
        let hi = change.old_range.end.min(updated.len()).max(lo);
        if change.new_text.as_str() != &updated[lo..hi] {
            updated.replace_range(lo..hi, &change.new_text);
            self.dispatch(
                AppCommand::UpdateQueryText {
                    tab_id,
                    pane_id: key.pane_id,
                    text: updated,
                },
                cx,
            );
        }
    }

    /// 获取（或惰性创建）Redis Workbench 的命令编辑器，并把文本变更同步回状态。
    fn redis_workbench_state(
        &mut self,
        tab_id: TabId,
        workbench: &RedisWorkbenchState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<editor_component::Editor> {
        let settings = self.controller.state().settings.clone();
        if let Some(editor) = self.redis_workbench_inputs.get(&tab_id) {
            let editor = editor.clone();
            // 执行 Run 后 workbench.text 被清空，需同步清掉编辑器残留文本。
            // 用户正常输入时编辑器与 workbench.text 通过 Changed 事件同步，
            // 不会出现「workbench.text 为空但编辑器非空且为正在输入」的情形。
            editor.update(cx, |editor, cx| {
                apply_code_editor_settings(editor, &settings, cx);
                if workbench.text.is_empty() && !editor.text().is_empty() {
                    editor.sync_text_silent("", cx);
                }
            });
            return editor;
        }

        // 构造 Redis 补全/执行/签名 adapter，注入通用编辑器。
        let adapter = std::sync::Arc::new(redis_editor_adapter::RedisAdapter::new());
        let providers = editor_component::Providers {
            completion: Some(adapter.clone() as _),
            execution: Some(adapter.clone() as _),
            signature: Some(adapter.clone() as _),
            ..Default::default()
        };
        let mut profile = fluxdb_editor_core::EditorProfile::default();
        profile.language_id = "redis".to_string();
        profile.show_line_numbers = true;
        profile.show_folding = false;
        profile.completion_trigger = fluxdb_editor_core::CompletionTrigger::Auto;
        profile.completion_trigger_chars = vec![' ', '.', '@'];
        profile.completion_min_prefix = 1;
        let config = fluxdb_editor_core::EditorConfig {
            profile,
            font: code_font_family(&settings),
            font_size: settings.editor_font_size.clamp(10, 24) as f32,
            line_height: settings.editor_line_height.clamp(13, 28) as f32,
            gutter_line_numbers: true,
        };
        let editor = cx.new(|cx| {
            editor_component::Editor::new(
                workbench.text.clone(),
                providers,
                Some(config),
                window,
                cx,
            )
        });
        // 按宿主当前主题名注入编辑器配色（theme_registry 派生，缺字段回退内置默认）。
        let editor_theme = self.editor_theme_for(cx);
        editor.update(cx, |editor, ecx| editor.set_theme(editor_theme, ecx));

        // 订阅编辑器事件：增量同步文本 + 执行命令。
        let subscription = cx.subscribe_in(
            &editor,
            window,
            move |this, editor, event, _window, cx| match event {
                editor_component::EditorEvent::Changed(change) => {
                    let text = editor.read(cx).text();
                    this.dispatch(
                        AppCommand::UpdateRedisWorkbenchText { tab_id, text },
                        cx,
                    );
                    let _ = change;
                }
                editor_component::EditorEvent::Execute { range, mode } => {
                    this.run_redis_workbench(tab_id, None, cx);
                    let _ = range;
                    let _ = mode;
                }
                _ => {}
            },
        );
        self.redis_workbench_inputs.insert(tab_id, editor.clone());
        self._redis_workbench_input_subscriptions
            .insert(tab_id, subscription);
        editor
    }

    /// 获取（或惰性创建）MongoDB 命令工作台的语句编辑器，并把文本变更同步回状态。
    ///
    /// 与 Redis Workbench 同构：一个多行编辑器 + Changed 同步 + Execute（Ctrl/Cmd+Enter）执行。
    /// 补全复用 CLI 的 shell 候选（库名 / 集合名 / 方法名），但不做语法高亮：
    /// shell 语句是「放宽了引号的 JSON + 方法链」，用 JSON 高亮会误染。
    pub(crate) fn mongo_workbench_state(
        &mut self,
        tab_id: TabId,
        workbench: &MongoWorkbenchState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<editor_component::Editor> {
        if let Some(adapter) = self.mongo_workbench_completion.get(&tab_id) {
            // 执行 `use <db>` 后工作台库会变，集合候选必须跟着换库。
            adapter.set_database(&workbench.database);
        }
        let settings = self.controller.state().settings.clone();
        if let Some(editor) = self.mongo_workbench_inputs.get(&tab_id) {
            let editor = editor.clone();
            // 执行后 workbench.text 被清空，需同步清掉编辑器残留文本（同 Redis Workbench）。
            editor.update(cx, |editor, cx| {
                apply_code_editor_settings(editor, &settings, cx);
                if workbench.text.is_empty() && !editor.text().is_empty() {
                    editor.sync_text_silent("", cx);
                }
            });
            return editor;
        }

        let mut profile = fluxdb_editor_core::EditorProfile::default();
        profile.language_id = "plain".to_string();
        profile.show_line_numbers = true;
        profile.show_folding = false;
        profile.completion_trigger = fluxdb_editor_core::CompletionTrigger::Auto;
        profile.completion_trigger_chars = vec!['.', '[', ' '];
        let config = fluxdb_editor_core::EditorConfig {
            profile,
            font: code_font_family(&settings),
            font_size: settings.editor_font_size.clamp(10, 24) as f32,
            line_height: settings.editor_line_height.clamp(13, 28) as f32,
            gutter_line_numbers: true,
        };
        // 连接已删除时仍可打开工作台（执行时才报错），但没有连接配置就不装补全，
        // 否则补全适配器会拿一份空配置去建连。
        let completion = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == workbench.connection_id)
            .map(|connection| {
                std::sync::Arc::new(MongoShellEditorAdapter::new(
                    connection.config.clone(),
                    workbench.database.clone(),
                ))
            });
        if let Some(adapter) = completion.as_ref() {
            self.mongo_workbench_completion
                .insert(tab_id, adapter.clone());
        }
        let providers = editor_component::Providers {
            completion: completion.map(|adapter| {
                adapter as std::sync::Arc<dyn fluxdb_editor_core::CompletionProvider>
            }),
            ..Default::default()
        };
        let editor = cx.new(|cx| {
            editor_component::Editor::new(
                workbench.text.clone(),
                providers,
                Some(config),
                window,
                cx,
            )
        });
        let editor_theme = self.editor_theme_for(cx);
        editor.update(cx, |editor, ecx| editor.set_theme(editor_theme, ecx));

        let subscription = cx.subscribe_in(
            &editor,
            window,
            move |this, editor, event, _window, cx| match event {
                editor_component::EditorEvent::Changed(_) => {
                    let text = editor.read(cx).text();
                    this.dispatch(
                        AppCommand::UpdateMongoWorkbenchText { tab_id, text },
                        cx,
                    );
                }
                editor_component::EditorEvent::Execute { .. } => {
                    this.run_mongo_workbench(tab_id, None, cx);
                }
                _ => {}
            },
        );
        self.mongo_workbench_inputs.insert(tab_id, editor.clone());
        self._mongo_workbench_input_subscriptions
            .insert(tab_id, subscription);
        editor
    }

    /// 获取（或惰性创建）查询页查找/替换输入框；文本变更时同步回编辑器 find 状态。
    #[allow(dead_code)] // 查找面板尚未接入宿主渲染，该方法及其两个 apply 一并预留。
    fn query_find_state(
        &mut self,
        key: QueryPaneKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<InputState>, Entity<InputState>) {
        let find = self.query_find_inputs.get(&key).cloned();
        let replace = self.query_replace_inputs.get(&key).cloned();
        if let (Some(find), Some(replace)) = (find, replace) {
            return (find, replace);
        }
        let find = if let Some(find) = self.query_find_inputs.get(&key).cloned() {
            find
        } else {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.data.placeholder.find")));
            let sub = cx.subscribe(&input, {
                move |this, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = input.read(cx).value().to_string();
                        this.apply_query_find_query(key, value, cx);
                    }
                }
            });
            self._query_find_input_subscriptions.insert(key, sub);
            self.query_find_inputs.insert(key, input.clone());
            input
        };
        let replace = if let Some(replace) = self.query_replace_inputs.get(&key).cloned() {
            replace
        } else {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.data.placeholder.replace")));
            let sub = cx.subscribe(&input, {
                move |this, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = input.read(cx).value().to_string();
                        this.apply_query_replace_text(key, value, cx);
                    }
                }
            });
            self._query_replace_input_subscriptions.insert(key, sub);
            self.query_replace_inputs.insert(key, input.clone());
            input
        };
        (find, replace)
    }

    /// 查找词变更：回写编辑器 find 状态并重算命中。
    #[allow(dead_code)] // 随 query_find_state 一并预留（查找面板未接入渲染）。
    fn apply_query_find_query(&mut self, key: QueryPaneKey, query: String, cx: &mut Context<Self>) {
        if let Some(editor) = self.query_editors.get(&key).cloned() {
            editor.update(cx, |editor, cx| editor.set_find_query(&query, cx));
        }
    }

    /// 替换词变更：回写编辑器 find 状态。
    #[allow(dead_code)] // 随 query_find_state 一并预留（查找面板未接入渲染）。
    fn apply_query_replace_text(&mut self, key: QueryPaneKey, replace: String, cx: &mut Context<Self>) {
        if let Some(editor) = self.query_editors.get(&key).cloned() {
            editor.update(cx, |editor, cx| editor.set_find_replace_text(&replace, cx));
        }
    }
}

fn apply_query_result_header_sort(
    sort_rules: &mut BTreeMap<QueryResultSortKey, Vec<DataSortRule>>,
    key: QueryResultSortKey,
    field: String,
    direction: Option<DataTableSortDirection>,
) {
    let current = sort_rules.get(&key).cloned().unwrap_or_default();
    let rules = data_sort_rules_after_header_sort(&current, field, direction);
    if rules.is_empty() {
        sort_rules.remove(&key);
    } else {
        sort_rules.insert(key, rules);
    }
}

fn data_table_columns_match(left: &[TableColumn], right: &[TableColumn]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.key == right.key && left.name == right.name && left.width == right.width)
}

fn data_table_rows_or_sorts_changed(
    left_page: &Arc<DataPage>,
    left_sorts: &[DataTableSort],
    right_page: &Arc<DataPage>,
    right_sorts: &[DataTableSort],
) -> bool {
    // 同一份显示页（缓存共享）只比排序；不同页才逐行比较。
    left_sorts != right_sorts
        || (!Arc::ptr_eq(left_page, right_page) && left_page.rows != right_page.rows)
}

fn query_result_cells_editable(has_result_editor: bool, sort_rules: &[DataSortRule]) -> bool {
    let _ = sort_rules;
    has_result_editor
}

/// 从控制器「已加载到内存」的状态中提取指定连接的 schema 上下文，供 SQL 补全注入。
///
/// 只读已加载数据，绝不触发任何网络 / 异步调用，也不触碰 controller 的 warmup /
/// `indexed_completion_tables`（那些可能访问数据库或阻塞 UI 线程）。
///
/// - `tables`：该连接已加载且 database 匹配的表 / 视图名（来自 `connections[].objects`）。
/// - `columns`：该连接已打开的 DataEditor 标签页中、database 匹配的列集合 `(表名, 列名)`
///   （来自 `tabs[].DataEditor.page.columns`）。
fn loaded_schema_context(
    state: &AppState,
    connection_id: ConnectionId,
    database: Option<&str>,
) -> sql_editor_adapter::SqlSchemaContext {
    let tables: Vec<String> = state
        .connections
        .iter()
        .find(|connection| connection.config.id == connection_id)
        .into_iter()
        .flat_map(|connection| connection.objects.iter())
        .filter(|object| matches!(object.path.kind, ObjectKind::Table | ObjectKind::View))
        .filter(|object| {
            database.is_none_or(|database| {
                object
                    .path
                    .database
                    .as_deref()
                    .is_some_and(|object_database| object_database.eq_ignore_ascii_case(database))
            })
        })
        .map(|object| object.path.name.clone())
        .collect();

    let columns: Vec<(String, String)> = state
        .tabs
        .iter()
        .filter_map(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if editor.object.connection_id == connection_id
                    && database.is_none_or(|database| {
                        editor
                            .object
                            .database
                            .as_deref()
                            .is_some_and(|object_database| object_database.eq_ignore_ascii_case(database))
                    }) =>
            {
                editor.page.as_ref().map(|page| (editor, page))
            }
            _ => None,
        })
        .flat_map(|(editor, page)| {
            page.columns.iter().map(|column| {
                (editor.object.name.clone(), column.name.clone())
            })
        })
        .collect();

    sql_editor_adapter::SqlSchemaContext::with_data(tables, columns)
}