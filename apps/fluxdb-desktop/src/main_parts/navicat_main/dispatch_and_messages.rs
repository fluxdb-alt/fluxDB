impl NavicatMain {
    fn dispatch(&mut self, command: AppCommand, cx: &mut Context<Self>) -> AppEvent {
        self.sync_settings_tab_dirty();
        if matches!(&command, AppCommand::ConfirmCloseDirtyTab(tab_id) if self.is_settings_tab(*tab_id)) {
            self.discard_settings_preview(cx);
        }
        let previous_query_history = self.controller.state().query_history.clone();
        let previous_redis_history = self.controller.state().redis_workbench_history.clone();
        let previous_tabs = self
            .controller
            .state()
            .tabs
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>();
        let event = self.controller.dispatch(command);
        self.apply_app_event(&event, cx);
        self.apply_closed_tabs(previous_tabs, cx);
        if self.controller.state().query_history != previous_query_history {
            let _ = self.storage.save_query_history(
                &self
                    .controller
                    .state()
                    .query_history
                    .iter()
                    .map(query_history_entry_to_record)
                    .collect::<Vec<_>>(),
            );
        }
        if self.controller.state().redis_workbench_history != previous_redis_history {
            let _ = self.storage.save_redis_workbench_history(
                &self
                    .controller
                    .state()
                    .redis_workbench_history
                    .iter()
                    .map(redis_workbench_entry_to_record)
                    .collect::<Vec<_>>(),
            );
        }
        cx.notify();
        event
    }

    fn is_settings_tab(&self, tab_id: TabId) -> bool {
        self.controller
            .state()
            .tabs
            .iter()
            .any(|tab| tab.id == tab_id && matches!(tab.kind, TabKind::Settings(_)))
    }

    fn sync_settings_tab_dirty(&mut self) {
        let Some(tab_id) = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| matches!(tab.kind, TabKind::Settings(_)))
            .map(|tab| tab.id)
        else {
            return;
        };
        let dirty = settings_changed(
            &self.controller.state().settings,
            &self.settings_editor_draft,
        );
        let current = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .is_some_and(|tab| tab.dirty);
        if current != dirty {
            let _ = self
                .controller
                .dispatch(AppCommand::SetTabDirty { tab_id, dirty });
        }
    }

    fn discard_settings_preview(&mut self, cx: &mut Context<Self>) {
        self.settings_editor_draft = self.controller.state().settings.clone();
        self.theme_mode = theme_mode_from_app_theme(self.settings_editor_draft.theme);
        self.preview_settings(cx);
    }

    /// 首帧后一次性恢复查询/工作台历史与保存的查询（不阻塞 open_window 前主线程建窗）。
    /// 首屏不依赖这三类数据（仅在打开对应面板/命令时读取），故由后台任务异步恢复，
    /// 完成后经 `cx.notify()` 补显。
    fn load_persisted_history_in_background(&mut self, cx: &mut Context<Self>) {
        if self.persisted_history_loaded {
            return;
        }
        self.persisted_history_loaded = true;
        let storage = self.storage.clone();
        let _ = cx.spawn(async move |view, cx| {
            let query_history = storage.load_query_history().ok();
            let redis_history = storage.load_redis_workbench_history().ok();
            let saved_queries = storage.load_saved_queries().ok();
            let Some(view) = view.upgrade() else {
                return;
            };
            let _ = view.update(cx, |this, cx| {
                if let Some(history) = query_history {
                    let _ = this.controller.dispatch(AppCommand::ReplaceQueryHistory(
                        history.into_iter().map(query_history_record_to_entry).collect(),
                    ));
                }
                // Redis Workbench 历史：无对应 AppCommand，沿用启动时的公共 trait
                // `WorkbenchHistoryStore::append_history` 按 scope（连接 + 逻辑库）逐条追加，
                // App 层会为其分配单调递增 ID，据此加载幂等。
                if let Some(history) = redis_history {
                    for record in history {
                        let scope = WorkbenchHistoryScope::Redis {
                            connection_id: record.connection_id,
                            database: record.database,
                        };
                        let item = WorkbenchHistoryItem {
                            id: record.id,
                            text: record.text,
                            success: record.success,
                            executed_at_unix_secs: record.executed_at_unix_secs,
                            summary: record.summary,
                            source: redis_workbench_source_from_str(&record.source),
                        };
                        this.controller.append_history(&scope, item);
                    }
                }
                if let Some(saved) = saved_queries {
                    this.saved_queries = saved;
                }
                cx.notify();
            });
        });
    }

    fn start_completion_index_warmup_for_tab(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some((connection_id, database)) = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::QueryEditor(editor) => Some((editor.connection_id, editor.database.clone())),
                _ => None,
            })
        else {
            return;
        };
        self.start_completion_index_warmup(connection_id, database, cx);
    }

    fn start_completion_index_warmup(
        &mut self,
        connection_id: ConnectionId,
        database: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !self.controller.state().settings.enable_completion_index {
            return;
        }
        let key = (connection_id, database.clone());
        if self._completion_index_tasks.contains_key(&key) {
            return;
        }

        let mut controller = self.controller.clone();
        let task_key = key.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::WarmCompletionIndex {
                        connection_id,
                        database,
                    }) {
                        AppEvent::CompletionIndexWarmed(_, _) => Ok(()),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "补全索引失败".to_string(),
                            message: "补全索引预热没有返回结果".to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, _cx| {
                    this._completion_index_tasks.remove(&task_key);
                    let _ = result;
                });
            });
        });
        self._completion_index_tasks.insert(key, task);
    }

    fn apply_closed_tabs(&mut self, previous_tabs: Vec<TabId>, cx: &mut Context<Self>) {
        for tab_id in previous_tabs {
            if self.controller.state().tabs.iter().any(|tab| tab.id == tab_id) {
                continue;
            }
            self.apply_app_event(&AppEvent::TabClosed(tab_id), cx);
        }
    }

    fn active_cell_detail_edit_value(&self, cx: &App) -> Option<String> {
        if self.cell_detail_input.read(cx).value().is_empty() {
            return self
                .controller
                .state()
                .active_tab()
                .and_then(|tab| match &tab.kind {
                    TabKind::DataEditor(editor) => Some(editor.cell_detail_panel.edit_value.clone()),
                    TabKind::QueryEditor(editor) => active_query_result_editor_state(editor)
                        .map(|editor| editor.cell_detail_panel.edit_value.clone()),
                    _ => None,
                });
        }

        Some(self.cell_detail_input.read(cx).value().to_string())
    }

    fn download_active_cell(&mut self, cx: &mut Context<Self>) {
        let Some((tab_id, row, column)) =
            self.controller
                .state()
                .active_tab()
                .and_then(|tab| match &tab.kind {
                    TabKind::DataEditor(editor) => {
                        let active_cell = editor.cell_detail_panel.active_cell?;
                        Some((tab.id, active_cell.row, active_cell.column))
                    }
                    TabKind::QueryEditor(editor) => {
                        let active_cell =
                            active_query_result_editor_state(editor)?.cell_detail_panel.active_cell?;
                        Some((tab.id, active_cell.row, active_cell.column))
                    }
                    _ => None,
                })
        else {
            return;
        };

        let key = (tab_id, row, column);
        if self._cell_binary_download_tasks.contains_key(&key) {
            self.show_message("正在下载二进制数据", AppMessageKind::Warning, cx);
            return;
        }

        let path = std::env::temp_dir().join(format!("gdb-cell-r{}-c{}.bin", row + 1, column + 1));
        let mut controller = self.controller.clone();
        self.show_message("正在下载二进制数据", AppMessageKind::Success, cx);
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    let bytes = match controller.dispatch(AppCommand::DownloadBinaryCell {
                        tab_id,
                        row,
                        column,
                    }) {
                        AppEvent::BinaryCellDownloaded { bytes, .. } => Ok(bytes),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "下载失败".to_string(),
                            message: "二进制下载没有返回结果".to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }?;

                    fs::write(&path, bytes)
                        .map(|_| path)
                        .map_err(|_| fluxdb_core::UserFacingError {
                            title: "下载失败".to_string(),
                            message: "写入临时文件失败".to_string(),
                            detail: None,
                            retryable: true,
                        })
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._cell_binary_download_tasks.remove(&key);
                    match result {
                        Ok(path) => {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                path.display().to_string(),
                            ));
                            this.show_message(
                                "已保存文件路径到剪贴板",
                                AppMessageKind::Success,
                                cx,
                            );
                        }
                        Err(error) => {
                            this.show_message(error.message, AppMessageKind::Error, cx);
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._cell_binary_download_tasks.insert(key, task);
    }

    fn upload_active_binary_cell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((tab_id, row, column, is_binary)) =
            self.controller
                .state()
                .active_tab()
                .and_then(|tab| match &tab.kind {
                    TabKind::DataEditor(editor) => {
                        let active_cell = editor.cell_detail_panel.active_cell?;
                        let value = editor
                            .page
                            .as_ref()?
                            .rows
                            .get(active_cell.row)?
                            .values
                            .get(active_cell.column)?;
                        Some((
                            tab.id,
                            active_cell.row,
                            active_cell.column,
                            matches!(value, CellValue::Bytes(_) | CellValue::BinarySummary(_)),
                        ))
                    }
                    TabKind::QueryEditor(editor) => {
                        let editor = active_query_result_editor_state(editor)?;
                        let active_cell = editor.cell_detail_panel.active_cell?;
                        let value = editor
                            .page
                            .as_ref()?
                            .rows
                            .get(active_cell.row)?
                            .values
                            .get(active_cell.column)?;
                        Some((
                            tab.id,
                            active_cell.row,
                            active_cell.column,
                            matches!(value, CellValue::Bytes(_) | CellValue::BinarySummary(_)),
                        ))
                    }
                    _ => None,
                })
        else {
            return;
        };
        if !is_binary {
            return;
        }

        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择要写入二进制字段的文件".into()),
        });
        self._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
            let selected = receiver.await;
            let path = match selected {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    let _ = cx.update(|_, cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.show_message(
                                format!("选择文件失败：{error}"),
                                AppMessageKind::Error,
                                cx,
                            );
                        });
                    });
                    None
                }
                Err(error) => {
                    let _ = cx.update(|_, cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.show_message(
                                format!("选择文件失败：{error}"),
                                AppMessageKind::Error,
                                cx,
                            );
                        });
                    });
                    None
                }
            };
            let Some(path) = path else {
                return;
            };

            let mut controller = view
                .upgrade()
                .map(|view| view.read_with(cx, |this, _| this.controller.clone()));
            let result = cx
                .background_spawn(async move {
                    let Some(mut controller) = controller.take() else {
                        return Err(fluxdb_core::UserFacingError {
                            title: "替换失败".to_string(),
                            message: "窗口已关闭".to_string(),
                            detail: None,
                            retryable: false,
                        });
                    };
                    let event = controller.dispatch(AppCommand::ReplaceBinaryCellFromFile {
                        tab_id,
                        row,
                        column,
                        path,
                    });
                    match event {
                        AppEvent::TabActivated(_) => Ok(controller),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "替换失败".to_string(),
                            message: "二进制替换没有返回结果".to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }
                })
                .await;
            let _ = cx.update(|_, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| match result {
                    Ok(controller) => {
                        this.controller = controller;
                        this.dispatch(
                            AppCommand::OpenCellDetail {
                                tab_id,
                                row,
                                column,
                            },
                            cx,
                        );
                        this.refresh_active_data_table(tab_id, cx);
                        this.show_message("已读取文件，提交后生效", AppMessageKind::Success, cx);
                    }
                    Err(error) => {
                        this.show_message(error.message, AppMessageKind::Error, cx);
                    }
                });
            });
        }));
    }

    fn set_active_binary_cell_null(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some((row, column)) = self.active_detail_cell_position() else {
            return;
        };
        self.dispatch(
            AppCommand::SetBinaryCellNull {
                tab_id,
                row,
                column,
            },
            cx,
        );
        self.dispatch(
            AppCommand::OpenCellDetail {
                tab_id,
                row,
                column,
            },
            cx,
        );
        self.refresh_active_data_table(tab_id, cx);
    }

    fn save_active_binary_hex_edit(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some((row, column)) = self.active_detail_cell_position() else {
            return;
        };
        let Some(value) = self.active_cell_detail_edit_value(cx) else {
            return;
        };
        let event = self.controller.dispatch(AppCommand::UpdateBinaryCell {
            tab_id,
            row,
            column,
            payload: BinaryUpdatePayload::Hex(value),
        });
        if let AppEvent::Failed(error) = &event {
            self.show_message(error.message.clone(), AppMessageKind::Warning, cx);
        }
        self.apply_app_event(&event, cx);
        if !matches!(event, AppEvent::Failed(_)) {
            self.dispatch(
                AppCommand::OpenCellDetail {
                    tab_id,
                    row,
                    column,
                },
                cx,
            );
            self.refresh_active_data_table(tab_id, cx);
        }
    }

    fn active_detail_cell_position(&self) -> Option<(usize, usize)> {
        self.controller
            .state()
            .active_tab()
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => editor
                    .cell_detail_panel
                    .active_cell
                    .map(|cell| (cell.row, cell.column)),
                TabKind::QueryEditor(editor) => editor
                    .active_result_editor
                    .and_then(|page_index| editor.result_editors.get(&page_index))
                    .and_then(|editor| editor.cell_detail_panel.active_cell)
                    .map(|cell| (cell.row, cell.column)),
                _ => None,
            })
    }

    fn show_message(
        &mut self,
        text: impl Into<String>,
        kind: AppMessageKind,
        cx: &mut Context<Self>,
    ) {
        let message = next_app_message(self.app_message.as_ref(), text, kind);
        let message_id = message.id;
        self.app_message = Some(message);
        self._app_message_task = Some(cx.spawn(async move |view, cx| {
            smol::Timer::after(APP_MESSAGE_DURATION).await;
            view.update(cx, move |this, cx| {
                if this
                    .app_message
                    .as_ref()
                    .is_some_and(|message| message.id == message_id)
                {
                    this.app_message = None;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }

    fn apply_app_event(&mut self, event: &AppEvent, cx: &mut Context<Self>) {
        match event {
            AppEvent::DataLoaded(tab_id, page) => {
                if self.is_redis_data_tab(*tab_id) {
                    self.redis_key_folder_visible_cache.remove(tab_id);
                    self.redis_data_refresh_times.insert(*tab_id, Instant::now());
                    self.ensure_redis_refresh_time_ticker(cx);
                    // 首次分录（打开标签 / 刷新）走默认分页，不经过 redis_key_list_load_more /
                    // reload_redis_key_list_for_mode，不会设置 redis_key_list_loaded。这里把「已扫描」
                    // 同步到本次返回的加载上限，避免状态栏长时间显示「已扫描 0」。
                    let current_loaded = self.redis_key_list_loaded(*tab_id);
                    if page.limit > current_loaded {
                        self.set_redis_key_list_loaded(*tab_id, page.limit);
                    }
                    // 页面加载后触发可见行元信息惰性补全（首屏只拿键名，这里补齐类型/大小/TTL）。
                    self.ensure_redis_key_metadata(*tab_id, cx);
                }
                self.refresh_data_table_state(*tab_id, page, cx);
                if self.pending_table_data_export_after_load == Some(*tab_id) {
                    self.pending_table_data_export_after_load = None;
                    self.open_table_data_export(*tab_id, cx);
                }
                if !self.data_editor_has_dirty_changes(*tab_id) {
                    self.data_change_sql_preview_tabs.remove(tab_id);
                    if self.pending_apply_data_changes == Some(*tab_id) {
                        self.pending_apply_data_changes = None;
                    }
                }
            }
            AppEvent::RedisKeyMetadataLoaded { tab_id } => {
                // 元信息已由控制器合并进 editor.page。这里重建表格委托（refresh_active_data_table）
                // 让 type/value/size/TTL 立即上屏——否则 page 变了但 table delegate 仍是旧数据，看起来
                // 就像「永远没加载出来」。随后再续补下一批可见行；`loaded` 已含本次合并的键，
                // 没有新的缺元信息可见行时 `ensure_redis_key_metadata` 自然终止，不会重复循环。
                self.refresh_active_data_table(*tab_id, cx);
                self.ensure_redis_key_metadata(*tab_id, cx);
            }
            AppEvent::TabOpened(tab_id) => {
                if !self.tab_order.contains(tab_id) {
                    self.tab_order.push(*tab_id);
                }
                self.start_data_page_load_if_needed(*tab_id, cx);
                self.start_completion_index_warmup_for_tab(*tab_id, cx);
            }
            AppEvent::TableInfoChanged(tab_id, tab) => {
                self.start_table_info_load_if_needed(*tab_id, *tab, cx);
                let page = self
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find(|state_tab| state_tab.id == *tab_id)
                    .and_then(|state_tab| match &state_tab.kind {
                        TabKind::DataEditor(editor) => editor.page.clone(),
                        _ => None,
                    });
                if let Some(page) = page {
                    self.refresh_data_table_state(*tab_id, &page, cx);
                }
            }
            AppEvent::QueryFinished(tab_id, execution) => {
                self.query_result_sort_rules
                    .retain(|key, _| key.tab_id != *tab_id);
                self.clear_query_result_display_pages_for_tab(*tab_id);
                if let Some(page) = execution.results.first() {
                    self.refresh_data_table_state(*tab_id, page, cx);
                }
                self.collapsed_query_outputs.remove(tab_id);
                self.query_output_tabs.insert(
                    *tab_id,
                    if query_execution_result_entry_count(
                        execution.results.len(),
                        &execution.summaries,
                    ) > 0
                    {
                        QueryOutputTab::Result(0)
                    } else {
                        QueryOutputTab::Summary
                    },
                );
            }
            AppEvent::QueryResultPageRefreshed {
                tab_id,
                result_index,
                page_index,
                page,
            } => {
                self.query_output_tabs
                    .insert(*tab_id, QueryOutputTab::Result(*result_index));
                let result_editor = self
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find(|tab| tab.id == *tab_id)
                    .and_then(|tab| match &tab.kind {
                        TabKind::QueryEditor(editor) => {
                            editor.result_editors.get(page_index).cloned()
                        }
                        _ => None,
                    });
                let display_page = result_editor
                    .as_ref()
                    .and_then(|editor| editor.page.clone())
                    .unwrap_or_else(|| page.clone());
                self.refresh_query_result_table_state(
                    *tab_id,
                    &display_page,
                    *page_index,
                    *result_index,
                    result_editor.as_ref(),
                    cx,
                );
            }
            AppEvent::QueryCompletionsLoaded(_tab_id, _request_seq, _result) => {
                // 新编辑器改由内部 provider 异步驱动补全，宿主不再分发 QueryCompletionsLoaded 事件；
                // 补全结果由编辑器内部请求与注入处理。此处仅保留分支以编译。
            }
            AppEvent::TabActivated(tab_id) => {
                self.sync_query_editor_text(*tab_id, cx);
                self.start_completion_index_warmup_for_tab(*tab_id, cx);
                // 激活标签切换时让 Redis 历史抽屉跟随当前 scope（connection_id + database），
                // 切换 / 关闭后不残留旧连接、旧库的历史数据。
                self.sync_redis_history_scope(cx);
                if !self.data_editor_has_dirty_changes(*tab_id) {
                    self.data_change_sql_preview_tabs.remove(tab_id);
                    if self.pending_apply_data_changes == Some(*tab_id) {
                        self.pending_apply_data_changes = None;
                    }
                }
            }
            AppEvent::TabClosed(tab_id) => {
                if self.hovered_tab == Some(*tab_id) {
                    self.hovered_tab = None;
                }
                // 关闭标签后也可能改变激活标签：让 Redis 历史抽屉同步到新 scope
                // （当前 Redis Workbench 关闭 → 无 Redis Workbench 激活则一并回收抽屉状态）。
                self.sync_redis_history_scope(cx);
                self.data_table_states.remove(tab_id);
                self._data_table_width_subscriptions.remove(tab_id);
                self.data_table_column_widths
                    .retain(|key, _| key.tab_id != *tab_id);
                self.query_editors.remove(tab_id);
                self.query_statement_statuses.remove(tab_id);
                self.query_save_targets.remove(tab_id);
                self._query_editor_subscriptions.remove(tab_id);
                self.redis_workbench_inputs.remove(tab_id);
                self._redis_workbench_input_subscriptions.remove(tab_id);
                // 关掉 Redis CLI 终端：终止 PTY 子进程并释放对应终端会话实体。
                if let Some(session) = self.terminal_sessions.remove(tab_id) {
                    session.update(cx, |comp, comp_cx| comp.shutdown(comp_cx));
                }
                // 关闭 Redis Pub/Sub 会话：随模型 drop 断开订阅连接，停止后续轮询派发。
                self.close_pubsub_session(*tab_id);
                // 上下分栏为全局状态，标签页关闭时一并复位拖动，避免残留中断态。
                self.redis_workbench_panel_resize_start = None;
                if self
                    .pending_dangerous_redis_command
                    .as_ref()
                    .is_some_and(|pending| pending.tab_id == *tab_id)
                {
                    self.pending_dangerous_redis_command = None;
                }
                self._query_execute_tasks.remove(&tab_id.0);
                self._query_completion_tasks
                    .retain(|(task_tab_id, _), _| task_tab_id != tab_id);
                self.query_output_tabs.remove(tab_id);
                self.collapsed_query_outputs.remove(tab_id);
                self.query_output_heights.remove(tab_id);
                self.query_output_widths.remove(tab_id);
                self.query_result_sort_rules
                    .retain(|key, _| key.tab_id != *tab_id);
                self.clear_query_result_display_pages_for_tab(*tab_id);
                if self
                    .query_output_resize_start
                    .is_some_and(|start| start.tab_id == *tab_id)
                {
                    self.query_output_resize_start = None;
                }
                self.data_change_sql_preview_tabs.remove(tab_id);
                self._data_load_tasks.remove(&tab_id.0);
                self._table_info_tasks
                    .retain(|(task_tab_id, _), _| task_tab_id != tab_id);
                self._user_admin_users_tasks.remove(tab_id);
                self._user_admin_grants_tasks.remove(tab_id);
                self._user_admin_member_grants_tasks.remove(tab_id);
                self._user_admin_apply_tasks.remove(tab_id);
                self._cell_binary_download_tasks
                    .retain(|(task_tab_id, _, _), _| task_tab_id != tab_id);
                self.data_filter_panels.remove(tab_id);
                self.data_filter_rules.remove(tab_id);
                self.data_sort_rules.remove(tab_id);
                self.data_filter_draft_rules.remove(tab_id);
                self.data_sort_draft_rules.remove(tab_id);
                self.redis_data_refresh_times.remove(tab_id);
                // Redis Key 列表展示模式 / 展开态 / 叶子选中 / hover 均为 tab 级状态，随 tab 关闭回收。
                self.redis_key_list_modes.remove(tab_id);
                self.redis_key_list_expanded.remove(tab_id);
                self.redis_key_list_selected_leaf.remove(tab_id);
                self.redis_key_list_folder_hovered.remove(tab_id);
                self.redis_key_list_uniform_scroll.remove(tab_id);
                self.redis_key_folder_visible_cache.remove(tab_id);
                self.redis_key_list_loaded.remove(tab_id);
                if self.redis_key_list_hovered_tab == Some(*tab_id) {
                    self.redis_key_list_hovered_tab = None;
                }
                self.redis_key_value_drafts
                    .retain(|(draft_tab_id, _), _| draft_tab_id != tab_id);
                self.redis_key_name_drafts
                    .retain(|(draft_tab_id, _), _| draft_tab_id != tab_id);
                self.redis_key_ttl_drafts
                    .retain(|(draft_tab_id, _), _| draft_tab_id != tab_id);
                self.redis_set_member_search_queries
                    .retain(|(search_tab_id, _), _| search_tab_id != tab_id);
                self.redis_set_member_search_pages
                    .retain(|(search_tab_id, _, _), _| search_tab_id != tab_id);
                self.redis_set_member_search_generation.remove(&tab_id.0);
                self.redis_set_member_search_debounce = None;
                self.redis_set_member_search_debounce_until = None;
                if self
                    .redis_set_member_search_active
                    .as_ref()
                    .is_some_and(|(active_tab_id, _)| active_tab_id == tab_id)
                {
                    self.redis_set_member_search_active = None;
                }
                if self
                    .redis_set_member_search_loading
                    .as_ref()
                    .is_some_and(|(loading_tab_id, _, _)| loading_tab_id == tab_id)
                {
                    self.redis_set_member_search_loading = None;
                }
                if self
                    .redis_set_member_search_more_loading
                    .as_ref()
                    .is_some_and(|(more_tab_id, _, _)| more_tab_id == tab_id)
                {
                    self.redis_set_member_search_more_loading = None;
                }
                if self
                    .redis_key_value_active
                    .as_ref()
                    .is_some_and(|(active_tab_id, _)| active_tab_id == tab_id)
                {
                    self.redis_key_value_active = None;
                }
                if self
                    .redis_key_meta_active
                    .as_ref()
                    .is_some_and(|(active_tab_id, _)| active_tab_id == tab_id)
                {
                    self.redis_key_meta_active = None;
                    self.redis_key_meta_editing = None;
                }
                if self
                    .pending_redis_key_delete
                    .as_ref()
                    .is_some_and(|pending| pending.tab_id == *tab_id)
                {
                    self.pending_redis_key_delete = None;
                }
                if self
                    .pending_redis_stream_entry_add
                    .as_ref()
                    .is_some_and(|pending| pending.tab_id == *tab_id)
                {
                    self.pending_redis_stream_entry_add = None;
                }
                if self
                    .pending_redis_stream_entry_delete
                    .as_ref()
                    .is_some_and(|pending| pending.tab_id == *tab_id)
                {
                    self.pending_redis_stream_entry_delete = None;
                }
                if self
                    .pending_redis_hash_field_drawer
                    .as_ref()
                    .is_some_and(|pending| pending.tab_id == *tab_id)
                {
                    self.pending_redis_hash_field_drawer = None;
                    self.redis_hash_field_rows.clear();
                    self.redis_hash_field_drawer_rows.clear();
                }
                if self
                    .pending_redis_zset_member_drawer
                    .as_ref()
                    .is_some_and(|pending| pending.tab_id == *tab_id)
                {
                    self.pending_redis_zset_member_drawer = None;
                    self.redis_zset_member_rows.clear();
                    self.redis_zset_member_drawer_rows.clear();
                }
                if self
                    .pending_redis_list_item_drawer
                    .as_ref()
                    .is_some_and(|pending| pending.tab_id == *tab_id)
                {
                    self.pending_redis_list_item_drawer = None;
                    self.redis_list_item_drawer_rows.clear();
                }
                self.redis_hash_field_search_queries
                    .retain(|(search_tab_id, _), _| search_tab_id != tab_id);
                self.redis_hash_field_search_pages
                    .retain(|(search_tab_id, _, _), _| search_tab_id != tab_id);
                self.redis_hash_field_search_generation.remove(&tab_id.0);
                self.redis_hash_field_search_debounce = None;
                self.redis_hash_field_search_debounce_until = None;
                if self
                    .redis_hash_field_search_active
                    .as_ref()
                    .is_some_and(|(active_tab_id, _)| active_tab_id == tab_id)
                {
                    self.redis_hash_field_search_active = None;
                }
                self.redis_zset_member_search_queries
                    .retain(|(search_tab_id, _), _| search_tab_id != tab_id);
                self.redis_zset_member_search_pages
                    .retain(|(search_tab_id, _, _), _| search_tab_id != tab_id);
                self.redis_zset_member_search_generation.remove(&tab_id.0);
                self.redis_zset_member_search_debounce = None;
                self.redis_zset_member_search_debounce_until = None;
                if self
                    .redis_zset_member_search_active
                    .as_ref()
                    .is_some_and(|(active_tab_id, _)| active_tab_id == tab_id)
                {
                    self.redis_zset_member_search_active = None;
                }
                self.redis_list_item_search_queries
                    .retain(|(search_tab_id, _), _| search_tab_id != tab_id);
                self.redis_list_item_search_pages
                    .retain(|(search_tab_id, _, _), _| search_tab_id != tab_id);
                self.redis_list_item_search_generation.remove(&tab_id.0);
                if self
                    .redis_list_item_search_active
                    .as_ref()
                    .is_some_and(|(active_tab_id, _)| active_tab_id == tab_id)
                {
                    self.redis_list_item_search_active = None;
                }
                self._redis_key_value_apply_tasks
                    .remove(&redis_key_value_apply_task_id(*tab_id));
                self._data_load_tasks
                    .remove(&redis_set_member_search_task_id(*tab_id));
                self._data_load_tasks
                    .remove(&redis_set_member_mutation_task_id(*tab_id));
                self._data_load_tasks
                    .remove(&redis_hash_field_search_task_id(*tab_id));
                self._data_load_tasks
                    .remove(&redis_hash_field_mutation_task_id(*tab_id));
                self._data_load_tasks
                    .remove(&redis_zset_member_search_task_id(*tab_id));
                self._data_load_tasks
                    .remove(&redis_zset_member_mutation_task_id(*tab_id));
                self._data_load_tasks
                    .remove(&redis_list_item_search_task_id(*tab_id));
                self._data_load_tasks
                    .remove(&redis_list_item_mutation_task_id(*tab_id));
                self.data_filter_modes.remove(tab_id);
                self.data_filter_texts.remove(tab_id);
                self.data_sort_texts.remove(tab_id);
                self.data_filter_panel_heights.remove(tab_id);
                if self
                    .data_filter_panel_resize_start
                    .is_some_and(|start| start.tab_id == *tab_id)
                {
                    self.data_filter_panel_resize_start = None;
                }
                self.data_filter_applying_tabs.remove(tab_id);
                self._data_filter_apply_tasks.remove(&tab_id.0);
                self.data_search_panels.remove(tab_id);
                self.data_search_queries.remove(tab_id);
                self.data_search_active_matches.remove(tab_id);
                self.data_search_highlight_all_tabs.remove(tab_id);
                if self
                    .data_cell_editing
                    .as_ref()
                    .is_some_and(|editing| editing.tab_id == *tab_id)
                {
                    self.data_cell_editing = None;
                }
                if self
                    .data_cell_context_menu
                    .as_ref()
                    .is_some_and(|menu| menu.tab_id == *tab_id)
                {
                    self.data_cell_context_menu = None;
                }
                if self.pending_apply_data_changes == Some(*tab_id) {
                    self.pending_apply_data_changes = None;
                }
                self.visible_table_fields.remove(tab_id);
                self.pinned_tabs.remove(tab_id);
                self.tab_order.retain(|order_tab_id| order_tab_id != tab_id);
                if self.field_filter_popover == Some(*tab_id) {
                    self.field_filter_popover = None;
                }
                if self
                    .data_filter_popover
                    .is_some_and(|popover| popover.tab_id == *tab_id)
                {
                    self.data_filter_popover = None;
                }
                self.sync_table_hover_overlay_block(cx);
            }
            _ => {}
        }
    }

}
