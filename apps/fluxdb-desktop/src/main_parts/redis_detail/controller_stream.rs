impl NavicatMain {
    fn open_redis_stream_entry_add(
        &mut self,
        tab_id: TabId,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_redis_stream_entry_add = Some(RedisStreamEntryAddForm { tab_id, key });
        self.redis_stream_entry_id_input.update(cx, |input, cx| {
            input.set_value("*", window, cx);
        });
        self.redis_stream_entry_field_rows = vec![new_redis_stream_entry_field_inputs(window, cx)];
        // MAXLEN 不沿用上一次的值：裁剪是破坏性操作，必须每次显式填写
        self.redis_stream_maxlen_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        // 每次打开抽屉从顶部开始，避免沿用上次的滚动偏移
        self.redis_stream_entry_drawer_scroll.set_offset(Point::default());
        cx.notify();
    }

    fn add_redis_stream_entry_field_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.redis_stream_entry_field_rows
            .push(new_redis_stream_entry_field_inputs(window, cx));
        // 新增行后滚到底部，保证新行可见
        self.redis_stream_entry_drawer_scroll.scroll_to_bottom();
        if let Some(row) = self.redis_stream_entry_field_rows.last() {
            row.field_input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn remove_redis_stream_entry_field_row(
        &mut self,
        row_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.redis_stream_entry_field_rows.len() <= 1
            || row_index >= self.redis_stream_entry_field_rows.len()
        {
            return;
        }
        self.redis_stream_entry_field_rows.remove(row_index);
        let focus_index = row_index.min(self.redis_stream_entry_field_rows.len() - 1);
        if let Some(row) = self.redis_stream_entry_field_rows.get(focus_index) {
            row.field_input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn request_redis_stream_entry_add(
        &mut self,
        tab_id: TabId,
        key: String,
        cx: &mut Context<Self>,
    ) {
        let id = self.redis_stream_entry_id_input.read(cx).value().to_string();
        let fields_snapshot = self.redis_stream_entry_fields_snapshot(cx);
        if let Some(message) = redis_stream_entry_id_validation_error(&id)
            .or_else(|| redis_stream_entry_fields_validation_error(&fields_snapshot))
        {
            self.show_message(message, AppMessageKind::Warning, cx);
            return;
        }
        let fields = match redis_stream_entry_field_pairs_from_snapshot(&fields_snapshot) {
            Ok(fields) => fields,
            Err(message) => {
                self.show_message(message, AppMessageKind::Warning, cx);
                return;
            }
        };
        // MAXLEN 留空表示不裁剪；填了必须是大于 0 的整数。
        let maxlen_input = self.redis_stream_maxlen_input.read(cx).value().to_string();
        let maxlen = match redis_stream_maxlen_from_input(&maxlen_input) {
            Ok(maxlen) => maxlen,
            Err(message) => {
                self.show_message(message, AppMessageKind::Warning, cx);
                return;
            }
        };
        self.start_redis_stream_entry_task(
            tab_id,
            key.clone(),
            AppCommand::AddRedisStreamEntry {
                tab_id,
                key,
                id,
                fields,
                maxlen,
            },
            true,
            cx,
        );
    }

    fn request_redis_stream_entry_delete(
        &mut self,
        tab_id: TabId,
        key: String,
        entry_id: String,
        cx: &mut Context<Self>,
    ) {
        self.start_redis_stream_entry_task(
            tab_id,
            key.clone(),
            AppCommand::DeleteRedisStreamEntry {
                tab_id,
                key,
                entry_id,
            },
            false,
            cx,
        );
    }

    fn start_redis_stream_entry_task(
        &mut self,
        tab_id: TabId,
        key: String,
        command: AppCommand,
        close_add: bool,
        cx: &mut Context<Self>,
    ) {
        let task_id = redis_stream_entry_apply_task_id(tab_id);
        if self._redis_key_value_apply_tasks.contains_key(&task_id) {
            return;
        }
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(command) {
                        AppEvent::DataLoaded(_, page) => {
                            page.rows.into_iter().next().ok_or_else(|| {
                                fluxdb_core::UserFacingError {
                                    title: "刷新失败".to_string(),
                                    message: "Redis Key 保存后没有返回数据".to_string(),
                                    detail: None,
                                    retryable: true,
                                }
                            })
                        }
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "保存失败".to_string(),
                            message: "Redis Stream 操作没有返回结果".to_string(),
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
                view.update(cx, |this, cx| {
                    this._redis_key_value_apply_tasks.remove(&task_id);
                    let event = this.controller.dispatch(AppCommand::FinishRedisKeyRefresh {
                        tab_id,
                        key: key.clone(),
                        result,
                    });
                    this.apply_app_event(&event, cx);
                    match event {
                        AppEvent::DataLoaded(_, _) => {
                            if close_add {
                                this.pending_redis_stream_entry_add = None;
                            }
                            this.pending_redis_stream_entry_delete = None;
                            // 条目增删后条目列表已变，从最新一页重新拉取。
                            this.rerun_redis_stream_entry_search(tab_id, key.clone(), cx);
                            // 消费者组的 pending/last-delivered 也会跟着变，展开中就一并刷新。
                            if this.redis_stream_groups_expanded {
                                this.request_redis_stream_groups(tab_id, key.clone(), cx);
                            }
                        }
                        AppEvent::Failed(error) => {
                            this.show_message(error.message, AppMessageKind::Error, cx);
                        }
                        _ => {}
                    }
                    cx.notify();
                });
            });
        });
        self._redis_key_value_apply_tasks.insert(task_id, task);
        cx.notify();
    }

    fn redis_stream_entry_fields_snapshot(&self, cx: &App) -> Vec<(String, String)> {
        self.redis_stream_entry_field_rows
            .iter()
            .map(|row| {
                (
                    row.field_input.read(cx).value().to_string(),
                    row.value_input.read(cx).value().to_string(),
                )
        })
            .collect()
    }

    // 发起 Stream 条目服务端分页查询：cursor 为 "" 表示取最新一页并替换，非 "" 表示「加载更多」向更旧方向追加。
    // 同一个 tab 同时只允许一个在飞查询（_data_load_tasks 去重），generation 用于丢弃切 key 后回来的旧结果。
    fn request_redis_stream_entry_search(
        &mut self,
        tab_id: TabId,
        key: String,
        cursor: String,
        cx: &mut Context<Self>,
    ) {
        let task_id = redis_stream_entry_search_task_id(tab_id);
        if self._data_load_tasks.contains_key(&task_id) {
            return;
        }
        let is_more = !cursor.is_empty();
        let generation = *self
            .redis_stream_entry_generation
            .entry(tab_id.0)
            .or_default();
        self.redis_stream_entry_loading = Some((tab_id, key.clone()));
        if is_more {
            self.redis_stream_entry_more_loading = Some((tab_id, key.clone()));
        }
        // 生效中的时间范围随请求一起下发，由服务端 XREVRANGE 过滤。
        let (since_ms, until_ms) = self
            .redis_stream_ranges
            .get(&(tab_id, key.clone()))
            .copied()
            .unwrap_or_default();
        let mut controller = self.controller.clone();
        let req_key = key.clone();
        let req_cursor = cursor.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadRedisStreamEntries {
                        tab_id,
                        key: req_key.clone(),
                        since_ms,
                        until_ms,
                        cursor: req_cursor.clone(),
                    }) {
                        AppEvent::RedisStreamEntriesLoaded {
                            key,
                            cursor,
                            entries,
                            next_cursor,
                            total,
                            ..
                        } => Ok((key, cursor, entries, next_cursor, total)),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "查询失败".to_string(),
                            message: "Stream 条目查询没有返回结果".to_string(),
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
                view.update(cx, |this, cx| {
                    this._data_load_tasks.remove(&task_id);
                    this.redis_stream_entry_loading = None;
                    this.redis_stream_entry_more_loading = None;
                    let still_current = this
                        .redis_stream_entry_generation
                        .get(&tab_id.0)
                        .copied()
                        .unwrap_or_default()
                        == generation;
                    if !still_current {
                        return;
                    }
                    match result {
                        Ok((key, cursor, entries, next_cursor, total)) => {
                            this.store_redis_stream_entry_page(
                                tab_id,
                                &key,
                                entries,
                                next_cursor,
                                total,
                                !cursor.is_empty(),
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
        self._data_load_tasks.insert(task_id, task);
        cx.notify();
    }

    // append 为 true 表示「加载更多」，把新一页追加到已有条目后面；否则整页替换。
    fn store_redis_stream_entry_page(
        &mut self,
        tab_id: TabId,
        key: &str,
        entries: Vec<(String, String, Vec<(String, String)>)>,
        next_cursor: String,
        total: usize,
        append: bool,
    ) {
        let rows = entries
            .into_iter()
            .map(|(id, time, fields)| RedisStreamEntryRow {
                id,
                time,
                fields: fields.into_iter().collect(),
            })
            .collect::<Vec<_>>();
        let page = self
            .redis_stream_entry_pages
            .entry((tab_id, key.to_string()))
            .or_default();
        page.total = total;
        page.next_cursor = next_cursor;
        if append {
            page.entries.extend(rows);
        } else {
            page.entries = rows;
        }
    }

    fn active_redis_stream_page(&self, tab_id: TabId, key: &str) -> Option<RedisStreamEntryPage> {
        self.redis_stream_entry_pages
            .get(&(tab_id, key.to_string()))
            .cloned()
    }

    // 应用时间范围过滤：解析两个输入框，写入生效范围后重拉首屏。
    fn apply_redis_stream_range(&mut self, tab_id: TabId, key: String, cx: &mut Context<Self>) {
        let since_text = self.redis_stream_since_input.read(cx).value().to_string();
        let until_text = self.redis_stream_until_input.read(cx).value().to_string();
        let since_ms = match redis_stream_time_from_input(&since_text) {
            Ok(value) => value,
            Err(message) => {
                self.show_message(format!("起始{message}"), AppMessageKind::Warning, cx);
                return;
            }
        };
        let until_ms = match redis_stream_time_from_input(&until_text) {
            Ok(value) => value,
            Err(message) => {
                self.show_message(format!("结束{message}"), AppMessageKind::Warning, cx);
                return;
            }
        };
        if let (Some(since), Some(until)) = (since_ms, until_ms)
            && since > until
        {
            self.show_message("起始时间不能晚于结束时间", AppMessageKind::Warning, cx);
            return;
        }
        self.redis_stream_ranges
            .insert((tab_id, key.clone()), (since_ms, until_ms));
        self.rerun_redis_stream_entry_search(tab_id, key, cx);
    }

    // 清除时间范围，回到「全部条目」。
    fn clear_redis_stream_range(
        &mut self,
        tab_id: TabId,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.redis_stream_ranges.remove(&(tab_id, key.clone()));
        self.redis_stream_since_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        self.redis_stream_until_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        self.rerun_redis_stream_entry_search(tab_id, key, cx);
    }

    // 展开/收起消费者组；首次展开时才去查 XINFO，避免平时多打两条命令。
    fn toggle_redis_stream_groups(&mut self, tab_id: TabId, key: String, cx: &mut Context<Self>) {
        self.redis_stream_groups_expanded = !self.redis_stream_groups_expanded;
        if self.redis_stream_groups_expanded {
            self.request_redis_stream_groups(tab_id, key, cx);
        }
        cx.notify();
    }

    fn request_redis_stream_groups(&mut self, tab_id: TabId, key: String, cx: &mut Context<Self>) {
        let task_id = redis_stream_groups_task_id(tab_id);
        if self._data_load_tasks.contains_key(&task_id) {
            return;
        }
        self.redis_stream_groups_loading = Some((tab_id, key.clone()));
        let mut controller = self.controller.clone();
        let req_key = key.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadRedisStreamGroups {
                        tab_id,
                        key: req_key.clone(),
                    }) {
                        AppEvent::RedisStreamGroupsLoaded { key, groups, .. } => Ok((key, groups)),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "查询失败".to_string(),
                            message: "Stream 消费者组查询没有返回结果".to_string(),
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
                view.update(cx, |this, cx| {
                    this._data_load_tasks.remove(&task_id);
                    this.redis_stream_groups_loading = None;
                    match result {
                        Ok((key, groups)) => {
                            this.redis_stream_groups.insert(
                                (tab_id, key),
                                groups
                                    .into_iter()
                                    .map(|(name, consumers, pending, last, detail)| {
                                        RedisStreamGroupRow {
                                            name,
                                            consumers,
                                            pending,
                                            last_delivered_id: last,
                                            consumer_detail: detail,
                                        }
                                    })
                                    .collect(),
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
        self._data_load_tasks.insert(task_id, task);
        cx.notify();
    }

    // 丢弃当前 Stream 查询结果并从最新一页重新拉取，用于「刷新」按钮和条目增删后的回源。
    fn rerun_redis_stream_entry_search(
        &mut self,
        tab_id: TabId,
        key: String,
        cx: &mut Context<Self>,
    ) {
        self.redis_stream_entry_generation
            .entry(tab_id.0)
            .and_modify(|generation| *generation += 1)
            .or_insert(0);
        self._data_load_tasks
            .remove(&redis_stream_entry_search_task_id(tab_id));
        self.redis_stream_entry_loading = None;
        self.redis_stream_entry_more_loading = None;
        self.request_redis_stream_entry_search(tab_id, key, String::new(), cx);
    }

    // 渲染期确保当前 (tab, key) 已拉过首屏；切换 key 时重新取最新一页。
    fn sync_redis_stream_entry_page(
        &mut self,
        tab_id: TabId,
        detail: &RedisKeyDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = (tab_id, detail.key.clone());
        if self.redis_stream_entry_active.as_ref() == Some(&active) {
            return;
        }
        self.redis_stream_entry_active = Some(active.clone());
        // 时间范围是按 (tab, key) 记的，但输入框只有一份：
        // 切 key 时必须回填成该 key 生效中的范围，否则框里显示的和实际过滤条件对不上。
        let (since_ms, until_ms) = self.redis_stream_ranges.get(&active).copied().unwrap_or_default();
        let since_text = since_ms.map(redis_stream_time_to_input).unwrap_or_default();
        let until_text = until_ms.map(redis_stream_time_to_input).unwrap_or_default();
        self.redis_stream_since_input.update(cx, |input, cx| {
            input.set_value(since_text, window, cx);
        });
        self.redis_stream_until_input.update(cx, |input, cx| {
            input.set_value(until_text, window, cx);
        });
        // 切换 key：作废上一 key 的在飞搜索并按新 key 重发（同其它类型面板口径，
        // 避免旧 key 的 Stream 查询失败回显、新 key 首搜被 per-tab 去重拦下）。
        self.rerun_redis_stream_entry_search(tab_id, detail.key.clone(), cx);
    }
}
