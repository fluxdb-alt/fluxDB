impl NavicatMain {
    fn redis_hash_field_drawer_rows_snapshot(&self, cx: &App) -> Vec<(String, String, String)> {
        self.redis_hash_field_drawer_rows
            .iter()
            .map(|row| {
                (
                    row.field_input.read(cx).value().to_string(),
                    row.value_input.read(cx).value().to_string(),
                    row.ttl_input.read(cx).value().to_string(),
                )
            })
            .collect()
    }

    fn open_redis_hash_field_add_drawer(
        &mut self,
        tab_id: TabId,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_redis_hash_field_delete = None;
        self.pending_redis_hash_field_drawer = Some(RedisHashFieldDrawerForm { tab_id, key });
        self.redis_hash_field_drawer_rows = vec![RedisHashFieldDrawerInputs {
            field_input: new_redis_hash_field_input(window, cx, None),
            value_input: new_redis_hash_value_input(window, cx, None),
            ttl_input: new_redis_hash_ttl_input(window, cx, None),
        }];
        self.redis_hash_field_drawer_scroll.set_offset(Point::default());
        cx.notify();
    }

    fn add_redis_hash_field_drawer_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.redis_hash_field_drawer_rows.push(RedisHashFieldDrawerInputs {
            field_input: new_redis_hash_field_input(window, cx, None),
            value_input: new_redis_hash_value_input(window, cx, None),
            ttl_input: new_redis_hash_ttl_input(window, cx, None),
        });
        self.redis_hash_field_drawer_scroll.scroll_to_bottom();
        if let Some(row) = self.redis_hash_field_drawer_rows.last() {
            row.field_input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn remove_redis_hash_field_drawer_row(
        &mut self,
        row_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if row_index >= self.redis_hash_field_drawer_rows.len() {
            return;
        }
        self.redis_hash_field_drawer_rows.remove(row_index);
        let focus_index = row_index.min(self.redis_hash_field_drawer_rows.len().saturating_sub(1));
        if let Some(row) = self.redis_hash_field_drawer_rows.get(focus_index) {
            row.field_input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn cancel_redis_hash_field_drawer(&mut self, cx: &mut Context<Self>) {
        self.pending_redis_hash_field_drawer = None;
        self.redis_hash_field_drawer_rows.clear();
        self.pending_redis_hash_field_delete = None;
        cx.notify();
    }

    fn confirm_redis_hash_field_drawer(
        &mut self,
        tab_id: TabId,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rows = self
            .redis_hash_field_drawer_rows_snapshot(cx)
            .into_iter()
            .map(|(field, value, ttl)| (field.trim().to_string(), value, ttl.trim().to_string()))
            .collect::<Vec<_>>();
        if rows.is_empty() {
            self.show_message("至少输入一个字段", AppMessageKind::Warning, cx);
            return;
        }
        if rows.iter().any(|(field, _, _)| field.is_empty()) {
            self.show_message("字段名不能为空", AppMessageKind::Warning, cx);
            return;
        }
        for (_, _, ttl) in &rows {
            if ttl.is_empty() {
                continue;
            }
            if ttl.parse::<u64>().ok().filter(|ttl| *ttl > 0).is_none() {
                self.show_message(
                    "Redis TTL 必须是秒数，且大于 0 秒或留空",
                    AppMessageKind::Warning,
                    cx,
                );
                return;
            }
        }
        self.pending_redis_hash_field_drawer = None;
        self.redis_hash_field_drawer_rows.clear();
        self.pending_redis_hash_field_delete = None;
        let commands = rows
            .into_iter()
            .map(|(field, value, ttl)| AppCommand::SetRedisHashField {
                tab_id,
                key: key.clone(),
                field,
                value,
                // 新增字段：留空即永不过期，上面已校验过秒数格式。
                ttl: match ttl.parse::<u64>() {
                    Ok(seconds) if seconds > 0 => RedisHashFieldTtl::Seconds(seconds),
                    _ => RedisHashFieldTtl::Persist,
                },
            })
            .collect::<Vec<_>>();
        self.start_redis_hash_field_mutation(tab_id, key, commands, cx);
        let _ = window;
    }

    fn request_redis_hash_field_search(
        &mut self,
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
        cx: &mut Context<Self>,
    ) {
        let task_id = redis_hash_field_search_task_id(tab_id);
        if self._data_load_tasks.contains_key(&task_id) {
            return;
        }
        let is_more = !cursor.is_empty();
        if is_more
            && self.redis_hash_field_search_more_loading.as_ref()
                == Some(&(tab_id, key.clone(), query.clone()))
        {
            return;
        }
        self.redis_hash_field_search_more_loading = None;
        if !is_more && self.pending_redis_hash_field_delete.take().is_some() {
            // 结果集即将刷新，撤销悬空的删除确认浮层
            cx.notify();
        }
        let generation = *self
            .redis_hash_field_search_generation
            .entry(tab_id.0)
            .or_default();
        let mut controller = self.controller.clone();
        let req_key = key.clone();
        let req_query = query.clone();
        let req_cursor = cursor.clone();
        let task = cx.spawn(async move |view, cx| {
            // 保留一份 req_key 用于结果回填时的「当前键」校验；req_key 会被内层后台任务 move 走。
            let still_current_key = req_key.clone();
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadRedisHashFields {
                        tab_id,
                        key: req_key.clone(),
                        query: req_query.clone(),
                        cursor: req_cursor.clone(),
                    }) {
                        AppEvent::RedisHashFieldsLoaded {
                            key,
                            query,
                            cursor,
                            fields,
                            next_cursor,
                            total,
                            ..
                        } => Ok((key, query, cursor, fields, next_cursor, total)),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "查询失败".to_string(),
                            message: "Hash 字段查询没有返回结果".to_string(),
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
                    this.redis_hash_field_search_loading = None;
                    this.redis_hash_field_search_more_loading = None;
                    // 代际不一致或搜索的 key 已不是当前 panel 的 key（用户已切走/残留事件误发）
                    // 时丢弃结果，避免「点 A 键却报 B 键错误」的跨 key 误报。
                    let still_current = this
                        .redis_hash_field_search_generation
                        .get(&tab_id.0)
                        .copied()
                        .unwrap_or_default()
                        == generation
                        && this.redis_hash_field_search_active.as_ref()
                            == Some(&(tab_id, still_current_key.clone()));
                    if !still_current {
                        return;
                    }
                    match result {
                        Ok((key, query, cursor, fields, next_cursor, total)) => {
                            this.store_redis_hash_field_search_page(
                                tab_id,
                                &key,
                                &query,
                                fields,
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
        self.redis_hash_field_search_loading = Some((tab_id, key.clone(), query.clone()));
        if is_more {
            self.redis_hash_field_search_more_loading = Some((tab_id, key.clone(), query.clone()));
        }
        self._data_load_tasks.insert(task_id, task);
        cx.notify();
    }

    fn store_redis_hash_field_search_page(
        &mut self,
        tab_id: TabId,
        key: &str,
        query: &str,
        fields: Vec<(String, String, String)>,
        next_cursor: String,
        total: usize,
        append: bool,
    ) {
        let page_key = (tab_id, key.to_string(), query.to_string());
        let page = self.redis_hash_field_search_pages.entry(page_key).or_default();
        page.total = total;
        page.next_cursor = next_cursor;
        if append {
            let mut existing = page
                .fields
                .iter()
                .map(|(field, _, _)| field.clone())
                .collect::<BTreeSet<_>>();
            for field in fields {
                if existing.insert(field.0.clone()) {
                    page.fields.push(field);
                }
            }
        } else {
            page.fields = fields;
        }
    }

    fn schedule_redis_hash_field_search(
        &mut self,
        tab_id: TabId,
        key: String,
        query: String,
        cx: &mut Context<Self>,
    ) {
        let until = Instant::now() + Duration::from_millis(350);
        self.redis_hash_field_search_debounce_until = Some(until);
        let req_tab_id = tab_id;
        let req_key = key.clone();
        let req_query = query.clone();
        self.redis_hash_field_search_debounce = Some(cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(350))
                .await;
            view.update(cx, move |this, cx| {
                let still_pending = this
                    .redis_hash_field_search_debounce_until
                    .is_some_and(|current_until| current_until == until);
                if still_pending {
                    this.redis_hash_field_search_debounce_until = None;
                    this.redis_hash_field_search_debounce = None;
                    // 防抖在「输入时」就抓了 key；350ms 后用户可能已切到别的键，
                    // 按旧键发搜索会对不再展示的旧键误发（如跨 key 的 WRONGTYPE 误报）。
                    // 校验当前 panel 的 active key 是否仍是捕获时的键，不是则丢弃。
                    let on_active_key = this.redis_hash_field_search_active.as_ref()
                        == Some(&(req_tab_id, req_key.clone()));
                    if on_active_key {
                        this.request_redis_hash_field_search(
                            req_tab_id,
                            req_key,
                            req_query,
                            String::new(),
                            cx,
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn rerun_redis_hash_field_search(&mut self, tab_id: TabId, key: String, cx: &mut Context<Self>) {
        // 删除 field / 保存写回等字段列表变更后自动关闭完整值内嵌面板（幂等）
        self.close_redis_hash_full_value_viewer(cx);
        let query = self
            .redis_hash_field_search_queries
            .get(&(tab_id, key.clone()))
            .cloned()
            .unwrap_or_default();
        self.redis_hash_field_search_generation
            .entry(tab_id.0)
            .and_modify(|generation| *generation += 1)
            .or_insert(0);
        self.redis_hash_field_search_debounce = None;
        self._data_load_tasks
            .remove(&redis_hash_field_search_task_id(tab_id));
        self.redis_hash_field_search_loading = None;
        self.redis_hash_field_search_more_loading = None;
        self.request_redis_hash_field_search(tab_id, key, query, String::new(), cx);
    }

    fn request_redis_hash_field_delete(
        &mut self,
        tab_id: TabId,
        key: String,
        field: String,
        cx: &mut Context<Self>,
    ) {
        let commands = vec![AppCommand::DeleteRedisHashField {
            tab_id,
            key: key.clone(),
            field,
        }];
        self.start_redis_hash_field_mutation(tab_id, key, commands, cx);
    }

    fn start_redis_hash_field_mutation(
        &mut self,
        tab_id: TabId,
        key: String,
        commands: Vec<AppCommand>,
        cx: &mut Context<Self>,
    ) {
        let task_id = redis_hash_field_mutation_task_id(tab_id);
        if self._data_load_tasks.contains_key(&task_id) {
            return;
        }
        self.redis_hash_field_search_generation
            .entry(tab_id.0)
            .and_modify(|generation| *generation += 1)
            .or_insert(0);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    let mut last_error: Option<fluxdb_core::UserFacingError> = None;
                    for command in commands {
                        match controller.dispatch(command) {
                            AppEvent::DataLoaded(_, _) => {}
                            AppEvent::Failed(error) => last_error = Some(error),
                            _ => {
                                return Err(fluxdb_core::UserFacingError {
                                    title: "保存失败".to_string(),
                                    message: "Hash 字段操作没有返回结果".to_string(),
                                    detail: None,
                                    retryable: true,
                                })
                            }
                        }
                    }
                    match last_error {
                        Some(error) => Err(error),
                        None => Ok(()),
                    }
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._data_load_tasks.remove(&task_id);
                    match result {
                        Ok(()) => this.rerun_redis_hash_field_search(tab_id, key, cx),
                        Err(error) => this.show_message(error.message, AppMessageKind::Error, cx),
                    }
                    cx.notify();
                });
            });
        });
        self._data_load_tasks.insert(task_id, task);
        cx.notify();
    }

    fn begin_redis_hash_field_edit(
        &mut self,
        target: RedisHashFieldEditingState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 完整值弹框打开期间禁止行内编辑：弹框保存后行数据刷新，
        // 若行内编辑进行中会以旧完整值为基底覆盖，造成 stale 覆盖。
        if self.redis_hash_full_value_viewer.borrow().is_some() {
            return;
        }
        if let Some(current) = self.redis_hash_field_editing.clone()
            && current != target
        {
            if !self.confirm_redis_hash_field_edit(cx) {
                return;
            }
        }
        if self.redis_hash_field_editing.as_ref().is_some_and(|current| current == &target) {
            let input = match target.kind {
                RedisHashFieldCellKind::Value => self.redis_hash_value_edit_input.clone(),
                RedisHashFieldCellKind::Ttl => self.redis_hash_ttl_edit_input.clone(),
            };
            input.update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        if self.redis_hash_field_editing.is_some() {
            return;
        }
        let Some(row) = self.redis_hash_field_rows.get(target.row_index).cloned() else {
            return;
        };
        // 兜底守卫（即使渲染路径漏了单元格守卫）：截断行默认拒绝编辑。
        // 值编辑在任何版本下都禁——截断片段回写会覆盖完整数据；
        // TTL 编辑仅当版本 ≥7.4（字段级 TTL 可用，纯 HPEXPIRE/HPERSIST 不重写 value）放行。
        if redis_hash_value_is_truncated(&row.value) {
            let version = self
                .redis_tab_connection_id(target.tab_id)
                .and_then(|connection_id| self.redis_server_versions.get(&connection_id).copied())
                .flatten();
            let ttl_edit_allowed = matches!(target.kind, RedisHashFieldCellKind::Ttl)
                && redis_hash_field_ttl_editable(version.as_ref(), true);
            if !ttl_edit_allowed {
                return;
            }
        }
        let input = match target.kind {
            RedisHashFieldCellKind::Value => self.redis_hash_value_edit_input.clone(),
            RedisHashFieldCellKind::Ttl => self.redis_hash_ttl_edit_input.clone(),
        };
        let value = match target.kind {
            RedisHashFieldCellKind::Value => row.value,
            RedisHashFieldCellKind::Ttl => redis_ttl_input_value(&redis_hash_field_ttl_display_value(
                row.ttl.as_str(),
            )),
        };
        self.redis_hash_field_editing = Some(target);
        self.redis_hash_field_hovered = None;
        input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn cancel_redis_hash_field_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(editing) = self.redis_hash_field_editing.take() else {
            return false;
        };
        if self.redis_hash_field_hovered.as_ref() == Some(&editing) {
            self.redis_hash_field_hovered = None;
        }
        cx.notify();
        true
    }

    fn confirm_redis_hash_field_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(target) = self.redis_hash_field_editing.clone() else {
            return false;
        };
        let Some(row) = self.redis_hash_field_rows.get(target.row_index).cloned() else {
            self.redis_hash_field_editing = None;
            cx.notify();
            return false;
        };
        let input = match target.kind {
            RedisHashFieldCellKind::Value => self.redis_hash_value_edit_input.clone(),
            RedisHashFieldCellKind::Ttl => self.redis_hash_ttl_edit_input.clone(),
        };
        let current = input.read(cx).value().to_string();
        let command = match target.kind {
            RedisHashFieldCellKind::Value => {
                // Value 允许前后空白和换行，必须精确比对，不能再 trim。
                if current == row.value {
                    self.cancel_redis_hash_field_edit(cx);
                    return true;
                }
                AppCommand::SetRedisHashField {
                    tab_id: target.tab_id,
                    key: target.key.clone(),
                    field: row.field,
                    value: current,
                    ttl: RedisHashFieldTtl::Keep,
                }
            }
            RedisHashFieldCellKind::Ttl => {
                let original = redis_ttl_input_value(&redis_hash_field_ttl_display_value(
                    row.ttl.as_str(),
                ));
                if current.trim() == original {
                    self.cancel_redis_hash_field_edit(cx);
                    return true;
                }
                let ttl = match redis_hash_field_ttl_command(&current, true) {
                    Ok(ttl) => ttl,
                    Err(message) => {
                        self.show_message(message, AppMessageKind::Warning, cx);
                        return false;
                    }
                };
                // 走纯 TTL 命令（HPEXPIRE/HPERSIST，Redis 7.4+ 字段级 TTL）：不携带 value，
                // 从根上避免「只改 TTL 却重写一遍 value」——对 >1MB 被截断的大字段尤其关键。
                AppCommand::SetRedisHashFieldTtl {
                    tab_id: target.tab_id,
                    key: target.key.clone(),
                    field: row.field,
                    ttl,
                }
            }
        };
        self.redis_hash_field_editing = None;
        self.redis_hash_field_hovered = None;
        self.start_redis_hash_field_mutation(target.tab_id, target.key, vec![command], cx);
        true
    }

    /// 打开完整值内嵌面板：初始化 viewer 并懒加载完整原始值（非截断 HGET）。
    /// 懒加载走 `_data_load_tasks`，与行内编辑共用 task 去重 id，避免同一 key 并发读写。
    fn open_redis_hash_full_value_viewer(
        &mut self,
        tab_id: TabId,
        key: String,
        field: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 同一目标已打开：无操作，避免重复加载
        if self
            .redis_hash_full_value_viewer
            .borrow()
            .as_ref()
            .is_some_and(|viewer| {
                viewer.tab_id == tab_id && viewer.key == key && viewer.field == field
            })
        {
            return;
        }
        // 切换目标：取消在飞的旧加载任务（drop Task 即取消），避免旧任务完成时覆盖新 viewer
        self._data_load_tasks
            .remove(&redis_hash_field_mutation_task_id(tab_id));
        *self.redis_hash_full_value_viewer.borrow_mut() = Some(RedisHashFullValueViewer {
            tab_id,
            key: key.clone(),
            field: field.clone(),
            full_value: None,
            loading: true,
            editing: false,
            saving: false,
            error: false,
        });
        let window_handle = window.window_handle();
        self.load_redis_hash_full_value(tab_id, key, field, window_handle, cx);
        cx.notify();
    }

    /// 懒加载完整原始值：dispatch `LoadRedisHashFieldFull`，成功后写入 viewer 并结束 loading。
    /// 失败弹错误 toast，viewer 保留以便重试（关闭按钮仍在）。
    fn load_redis_hash_full_value(
        &mut self,
        tab_id: TabId,
        key: String,
        field: String,
        window_handle: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let task_id = redis_hash_field_mutation_task_id(tab_id);
        if self._data_load_tasks.contains_key(&task_id) {
            return;
        }
        let mut viewer_ref = self.redis_hash_full_value_viewer.borrow_mut();
        let Some(viewer) = viewer_ref.as_mut() else {
            return;
        };
        viewer.loading = true;
        viewer.error = false;
        let mut controller = self.controller.clone();
        // 任务期目标（tab+field）：完成时核对 viewer 仍指向它，防止切换目标后旧任务误写
        let task_field = field.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    controller.dispatch(AppCommand::LoadRedisHashFieldFull {
                        tab_id,
                        key,
                        field,
                    })
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._data_load_tasks.remove(&task_id);
                    match result {
                        // 没有返回数据：按加载失败处理，面板内展示错误 + 重试
                        // 仅当 viewer 仍指向本任务的 field 时更新（切换目标后旧任务过期）
                        AppEvent::DataLoaded(_, _) => {
                            if this.redis_hash_full_value_viewer.borrow().as_ref().is_some_and(|v| v.tab_id == tab_id && v.field == task_field) {
                                if let Some(viewer) = this.redis_hash_full_value_viewer.borrow_mut().as_mut() {
                                    viewer.loading = false;
                                    viewer.error = true;
                                }
                            }
                        }
                        AppEvent::RedisHashFieldFullValueLoaded { tab_id: tid, field: f, value } => {
                            if this
                                .redis_hash_full_value_viewer
                                .borrow()
                                .as_ref()
                                .is_some_and(|v| v.tab_id == tid && v.field == f)
                            {
                                if let Some(viewer) = this
                                    .redis_hash_full_value_viewer
                                    .borrow_mut()
                                    .as_mut()
                                {
                                    viewer.full_value = Some(value);
                                    viewer.loading = false;
                                    viewer.error = false;
                                }
                                let Some(value) = this
                                    .redis_hash_full_value_viewer
                                    .borrow()
                                    .as_ref()
                                    .and_then(|viewer| viewer.full_value.clone())
                                else {
                                    return;
                                };
                                let input = this.redis_hash_value_edit_input.clone();
                                let _ = window_handle.update(cx, |_, window, cx| {
                                    input.update(cx, |input, cx| {
                                        input.set_value(value, window, cx);
                                    });
                                });
                            }
                        }
                        AppEvent::Failed(error) => {
                            // 失败时结束 loading 并标记 error，面板内展示错误 + 重试
                            // 仅当 viewer 仍指向本任务的 field 时更新（切换目标后旧任务过期）
                            if this.redis_hash_full_value_viewer.borrow().as_ref().is_some_and(|v| v.tab_id == tab_id && v.field == task_field) {
                                if let Some(viewer) = this.redis_hash_full_value_viewer.borrow_mut().as_mut() {
                                    viewer.loading = false;
                                    viewer.error = true;
                                }
                            }
                            this.show_message(error.message, AppMessageKind::Error, cx);
                        }
                        _ => {}
                    }
                    cx.notify();
                });
            });
        });
        self._data_load_tasks.insert(task_id, task);
        cx.notify();
    }

    /// 进入编辑态：把已加载的完整值注入共享多行输入框。
    fn begin_redis_hash_full_value_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut viewer_ref = self.redis_hash_full_value_viewer.borrow_mut();
        let Some(viewer) = viewer_ref.as_mut() else {
            return;
        };
        if viewer.loading || viewer.editing || viewer.saving {
            return;
        }
        let Some(value) = viewer.full_value.clone() else {
            return;
        };
        viewer.editing = true;
        self.redis_hash_value_edit_input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// 取消编辑态：保留已编辑内容在输入框（下次进入会覆盖），只退出编辑态不关闭弹框。
    fn cancel_redis_hash_full_value_edit(&mut self, cx: &mut Context<Self>) {
        let mut viewer_ref = self.redis_hash_full_value_viewer.borrow_mut();
        let Some(viewer) = viewer_ref.as_mut() else {
            return;
        };
        if !viewer.editing {
            return;
        }
        viewer.editing = false;
        cx.notify();
    }

    /// 保存编辑后的完整值：走 `SetRedisHashFieldRaw`（放行 >1MB 截断值），
    /// 成功写回后重跑字段搜索并关闭内嵌面板。
    fn save_redis_hash_full_value(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.redis_hash_full_value_viewer.borrow().clone() else {
            return;
        };
        if !viewer.editing || viewer.saving {
            return;
        }
        if self._data_load_tasks.contains_key(&redis_hash_field_mutation_task_id(viewer.tab_id)) {
            return;
        }
        let value = self.redis_hash_value_edit_input.read(cx).value().to_string();
        if let Some(viewer) = self.redis_hash_full_value_viewer.borrow_mut().as_mut() {
            viewer.saving = true;
        }
        let commands = vec![AppCommand::SetRedisHashFieldRaw {
            tab_id: viewer.tab_id,
            key: viewer.key.clone(),
            field: viewer.field.clone(),
            value,
            ttl: RedisHashFieldTtl::Keep,
        }];
        let done = move |this: &mut Self, cx: &mut Context<Self>| {
            *this.redis_hash_full_value_viewer.borrow_mut() = None;
            this.cancel_redis_hash_field_edit(cx);
            cx.notify();
        };
        self.run_redis_hash_full_value_save(viewer.tab_id, viewer.key, commands, done, cx);
    }

    /// 执行完整值保存 task：成功后重跑字段搜索（截断标记随新值刷新）并关闭内嵌面板。
    fn run_redis_hash_full_value_save(
        &mut self,
        tab_id: TabId,
        key: String,
        commands: Vec<AppCommand>,
        done: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let task_id = redis_hash_field_mutation_task_id(tab_id);
        if self._data_load_tasks.contains_key(&task_id) {
            return;
        }
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    let mut last_error: Option<fluxdb_core::UserFacingError> = None;
                    for command in commands {
                        match controller.dispatch(command) {
                            AppEvent::DataLoaded(_, _) => {}
                            AppEvent::Failed(error) => last_error = Some(error),
                            _ => {
                                return Err(fluxdb_core::UserFacingError {
                                    title: "保存失败".to_string(),
                                    message: "Hash 完整值保存没有返回结果".to_string(),
                                    detail: None,
                                    retryable: true,
                                })
                            }
                        }
                    }
                    match last_error {
                        Some(error) => Err(error),
                        None => Ok(()),
                    }
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._data_load_tasks.remove(&task_id);
                    match result {
                        Ok(()) => {
                            this.rerun_redis_hash_field_search(tab_id, key, cx);
                            done(this, cx);
                        }
                        Err(error) => {
                            if let Some(viewer) = this.redis_hash_full_value_viewer.borrow_mut().as_mut() {
                                viewer.saving = false;
                            }
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

    /// 关闭完整值内嵌面板并清理编辑态；同时取消在飞的完整值加载任务。
    fn close_redis_hash_full_value_viewer(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.redis_hash_full_value_viewer.borrow().clone() else {
            return;
        };
        // 取消在飞的加载任务：避免完成时误写状态，也避免遗留去重占位阻塞下次打开
        self._data_load_tasks
            .remove(&redis_hash_field_mutation_task_id(viewer.tab_id));
        *self.redis_hash_full_value_viewer.borrow_mut() = None;
        self.cancel_redis_hash_field_edit(cx);
        cx.notify();
    }

    fn active_redis_hash_page(&self, tab_id: TabId, key: &str) -> Option<RedisHashFieldPage> {
        let query = self
            .redis_hash_field_search_queries
            .get(&(tab_id, key.to_string()))
            .cloned()
            .unwrap_or_default();
        self.redis_hash_field_search_pages
            .get(&(tab_id, key.to_string(), query))
            .cloned()
    }

    fn sync_redis_hash_field_inputs(
        &mut self,
        tab_id: TabId,
        detail: &RedisKeyDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = (tab_id, detail.key.clone());
        let expected = self
            .redis_hash_field_search_queries
            .get(&active)
            .cloned()
            .unwrap_or_default();
        let active_changed = self.redis_hash_field_search_active.as_ref() != Some(&active);
        // 切换 key：作废上一 key 的在飞搜索并按新 key 重发首屏（与 Set 面板同口径，防止
        // 旧 key 的查询失败/结果回显到新 key 面板、且新 key 首搜被 per-tab 去重拦下）。
        if active_changed {
            self.rerun_redis_hash_field_search(tab_id, detail.key.clone(), cx);
        }
        let focused = self
            .redis_hash_field_search_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let current = self.redis_hash_field_search_input.read(cx).value().to_string();
        if active_changed || (!focused && current != expected) {
            self.redis_hash_field_search_syncing = true;
            self.redis_hash_field_search_input.update(cx, |input, cx| {
                input.set_value(expected, window, cx);
            });
            self.redis_hash_field_search_syncing = false;
        }
        if self
            .redis_hash_field_editing
            .as_ref()
            .is_some_and(|editing| editing.tab_id != tab_id || editing.key != detail.key)
        {
            self.redis_hash_field_editing = None;
        }
        if self
            .redis_hash_field_hovered
            .as_ref()
            .is_some_and(|hovered| hovered.tab_id != tab_id || hovered.key != detail.key)
        {
            self.redis_hash_field_hovered = None;
        }
        let rows = self
            .active_redis_hash_page(tab_id, &detail.key)
            .map(|page| {
                page.fields
                    .into_iter()
                    .map(|(field, value, ttl)| RedisHashFieldRow { field, value, ttl })
                    .collect::<Vec<_>>()
            });
        if let Some(rows) = rows
            && (self.redis_hash_field_rows.len() != rows.len()
                || self.redis_hash_field_rows != rows)
        {
            self.redis_hash_field_rows = rows;
        }
        // 记录已同步的 (tab, key)，避免每帧重复触发 HSCAN/HPTTL 查询。
        self.redis_hash_field_search_active = Some(active);
    }
}
