impl NavicatMain {
    /// 开始 List 元素行内编辑（LSET）。
    /// - 若已有其它行编辑中，先确认（提交）旧编辑再切换；
    /// - 目标是当前编辑行则聚焦输入框返回；
    /// - 从当前页 `items[row_index]` 取出 (绝对下标, 当前值) 作为编辑基底。
    fn begin_redis_list_item_edit(
        &mut self,
        target: RedisListItemEditingState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(current) = self.redis_list_item_editing.clone()
            && current != target
        {
            if !self.confirm_redis_list_item_edit(cx) {
                return;
            }
        }
        if self
            .redis_list_item_editing
            .as_ref()
            .is_some_and(|current| current == &target)
        {
            self.redis_list_value_edit_input
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        if self.redis_list_item_editing.is_some() {
            return;
        }
        let Some(item) = self
            .active_redis_list_page(target.tab_id, &target.key)
            .and_then(|page| page.items.get(target.row_index).cloned())
        else {
            return;
        };
        let (_index, value) = item;
        self.redis_list_item_editing = Some(target);
        self.redis_list_item_hovered = None;
        self.redis_list_value_edit_input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn cancel_redis_list_item_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(editing) = self.redis_list_item_editing.take() else {
            return false;
        };
        if self.redis_list_item_hovered.as_ref() == Some(&editing) {
            self.redis_list_item_hovered = None;
        }
        cx.notify();
        true
    }

    /// 确认 List 元素编辑：值与初始一致则取消（无写入）；否则发 LSET。
    fn confirm_redis_list_item_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(target) = self.redis_list_item_editing.clone() else {
            return false;
        };
        let Some((index, old_value)) = self
            .active_redis_list_page(target.tab_id, &target.key)
            .and_then(|page| page.items.get(target.row_index).cloned())
        else {
            self.redis_list_item_editing = None;
            cx.notify();
            return false;
        };
        let current = self
            .redis_list_value_edit_input
            .read(cx)
            .value()
            .to_string();
        if current == old_value {
            self.cancel_redis_list_item_edit(cx);
            return true;
        }
        self.redis_list_item_editing = None;
        self.redis_list_item_hovered = None;
        self.start_redis_list_item_mutation(
            target.tab_id,
            target.key.clone(),
            vec![AppCommand::SetRedisListItem {
                tab_id: target.tab_id,
                key: target.key,
                index,
                expected_old: Some(old_value),
                value: current,
            }],
            cx,
        );
        true
    }

    fn redis_list_item_drawer_rows_snapshot(&self, cx: &App) -> Vec<String> {
        self.redis_list_item_drawer_rows
            .iter()
            .map(|row| row.value_input.read(cx).value().to_string())
            .collect()
    }

    fn open_redis_list_item_add_drawer(
        &mut self,
        tab_id: TabId,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_redis_list_item_drawer = Some(RedisListItemDrawerForm { tab_id, key });
        self.redis_list_item_drawer_rows = vec![RedisListItemInputs {
            value_input: new_redis_list_item_input(window, cx, None),
        }];
        cx.notify();
    }

    fn add_redis_list_item_drawer_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.redis_list_item_drawer_rows.push(RedisListItemInputs {
            value_input: new_redis_list_item_input(window, cx, None),
        });
        self.redis_list_item_drawer_scroll.scroll_to_bottom();
        if let Some(row) = self.redis_list_item_drawer_rows.last() {
            row.value_input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn remove_redis_list_item_drawer_row(
        &mut self,
        row_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if row_index >= self.redis_list_item_drawer_rows.len() {
            return;
        }
        self.redis_list_item_drawer_rows.remove(row_index);
        let focus_index = row_index.min(self.redis_list_item_drawer_rows.len().saturating_sub(1));
        if let Some(row) = self.redis_list_item_drawer_rows.get(focus_index) {
            row.value_input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn cancel_redis_list_item_drawer(&mut self, cx: &mut Context<Self>) {
        self.pending_redis_list_item_drawer = None;
        self.redis_list_item_drawer_rows.clear();
        cx.notify();
    }

    fn confirm_redis_list_item_drawer(
        &mut self,
        tab_id: TabId,
        key: String,
        head: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = self
            .redis_list_item_drawer_rows_snapshot(cx)
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>();
        if items.is_empty() {
            self.show_message("至少输入一个元素", AppMessageKind::Warning, cx);
            return;
        }
        self.pending_redis_list_item_drawer = None;
        self.redis_list_item_drawer_rows.clear();
        self.start_redis_list_item_mutation(
            tab_id,
            key.clone(),
            vec![AppCommand::PushRedisListItems {
                tab_id,
                key,
                items,
                head,
            }],
            cx,
        );
        let _ = window;
    }

    /// 打开 List「删除元素」抽屉：位置下拉重置为默认第一项（Remove from tail，即 RPOP），数量默认 1。
    fn open_redis_list_item_remove_drawer(
        &mut self,
        tab_id: TabId,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.redis_list_item_remove_drawer = Some(RedisListItemRemoveDrawerForm { tab_id, key });
        self.redis_list_item_remove_confirm = None;
        // 位置下拉重置为第一项（Remove from tail）；head 在确认时由 selected_value 决定。
        self.redis_list_item_remove_select
            .update(cx, |select, cx| {
                select.set_selected_index(Some(IndexPath::new(0)), window, cx);
            });
        self.redis_list_item_remove_count_input.update(cx, |input, cx| {
            input.set_value("1", window, cx);
        });
        self.redis_list_item_remove_count_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn cancel_redis_list_item_remove_drawer(&mut self, cx: &mut Context<Self>) {
        self.redis_list_item_remove_drawer = None;
        self.redis_list_item_remove_confirm = None;
        cx.notify();
    }

    /// 点击「删除」：解析方向与数量并校验，通过后仅记录二次确认目标（待确认浮层），不立即删除。
    fn open_redis_list_item_remove_confirm(
        &mut self,
        tab_id: TabId,
        key: String,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.redis_list_item_remove_drawer.as_ref() else {
            return;
        };
        if form.tab_id != tab_id || form.key != key {
            return;
        }
        // 由位置下拉决定弹出方向：选中「Remove from head」→ 从头（LPOP），否则从尾（RPOP）。
        let head = self
            .redis_list_item_remove_select
            .read(cx)
            .selected_value()
            .is_some_and(|v| v.as_str() == REDIS_LIST_REMOVE_FROM_HEAD);
        let count_text = self
            .redis_list_item_remove_count_input
            .read(cx)
            .value()
            .to_string();
        let count = match count_text.trim().parse::<usize>() {
            Ok(count) if count > 0 => count,
            _ => {
                self.show_message("删除数量必须是大于 0 的整数", AppMessageKind::Warning, cx);
                return;
            }
        };
        self.redis_list_item_remove_confirm =
            Some(RedisListItemRemoveConfirmTarget { tab_id, key, head, count });
        cx.notify();
    }

    /// 确认浮层确认后真正执行删除：关闭抽屉并发 PopRedisListItems（LPOP/RPOP），成功后重拉首屏。
    fn apply_redis_list_item_remove(
        &mut self,
        tab_id: TabId,
        key: String,
        head: bool,
        count: usize,
        cx: &mut Context<Self>,
    ) {
        // 与新增抽屉一致：确认后立即关闭抽屉，删除结果由首屏刷新呈现。
        self.redis_list_item_remove_drawer = None;
        self.redis_list_item_remove_confirm = None;
        self.start_redis_list_item_mutation(
            tab_id,
            key.clone(),
            vec![AppCommand::PopRedisListItems {
                tab_id,
                key,
                head,
                count,
            }],
            cx,
        );
    }

    // 发起 List 元素服务端分页查询：cursor 为 "" 表示替换首屏，非 "" 表示追加「加载更多」。
    // query 是搜索词，随请求下发给连接器：非空时按「下标跳转」（LINDEX）读取单个元素，空串时分页 LRANGE。
    fn request_redis_list_item_search(
        &mut self,
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
        cx: &mut Context<Self>,
    ) {
        let task_id = redis_list_item_search_task_id(tab_id);
        if self._data_load_tasks.contains_key(&task_id) {
            return;
        }
        let is_more = !cursor.is_empty();
        let generation = *self
            .redis_list_item_search_generation
            .entry(tab_id.0)
            .or_default();
        self.redis_list_item_search_loading = Some((tab_id, key.clone(), cursor.clone()));
        if is_more {
            self.redis_list_item_search_more_loading =
                Some((tab_id, key.clone(), cursor.clone()));
        }
        let mut controller = self.controller.clone();
        let req_key = key.clone();
        // 用于 WRONGTYPE 后触发键详情重探测的独立克隆，避免与 req_key（被在飞查询拿走）冲突。
        let recovery_key = key.clone();
        let req_query = query.clone();
        let req_cursor = cursor.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadRedisListItems {
                        tab_id,
                        key: req_key.clone(),
                        query: req_query.clone(),
                        cursor: req_cursor.clone(),
                    }) {
                        AppEvent::RedisListItemsLoaded {
                            key,
                            query,
                            cursor,
                            items,
                            next_cursor,
                            total,
                            ..
                        } => Ok((key, query, cursor, items, next_cursor, total)),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "查询失败".to_string(),
                            message: "List 元素查询没有返回结果".to_string(),
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
                    this.redis_list_item_search_loading = None;
                    this.redis_list_item_search_more_loading = None;
                    let still_current = this
                        .redis_list_item_search_generation
                        .get(&tab_id.0)
                        .copied()
                        .unwrap_or_default()
                        == generation;
                    if !still_current {
                        return;
                    }
                    match result {
                        Ok((key, query, cursor, items, next_cursor, total)) => {
                            this.store_redis_list_item_search_page(
                                tab_id,
                                &key,
                                &query,
                                items,
                                next_cursor,
                                total,
                                !cursor.is_empty(),
                            );
                        }
                        Err(error) => {
                            this.show_message(error.message.clone(), AppMessageKind::Error, cx);
                            // WRONGTYPE 说明该键在服务端已不是 List（常因被删除后重建为其他类型、
                            // 或密钥列表里缓存的类型已过期）。只弹提示会一直卡在错误的 List 面板上，
                            // 所以额外触发一次键详情重探测，让面板切回真实类型。
                            if error.message.contains("不是 List") {
                                this.request_redis_key_refresh(
                                    tab_id,
                                    recovery_key.clone(),
                                    RedisKeyDetailRefreshKind::Key,
                                    cx,
                                );
                            }
                        }
                    }
                    cx.notify();
                });
            });
        });
        self.redis_list_item_search_loading = Some((tab_id, key.clone(), cursor.clone()));
        self._data_load_tasks.insert(task_id, task);
        cx.notify();
    }

    // append 为 true 表示「加载更多」，把新一页追加到已有元素后面；否则整页替换。
    fn store_redis_list_item_search_page(
        &mut self,
        tab_id: TabId,
        key: &str,
        query: &str,
        items: Vec<(usize, String)>,
        next_cursor: String,
        total: usize,
        append: bool,
    ) {
        let page_key = (tab_id, key.to_string(), query.to_string());
        let page = self.redis_list_item_search_pages.entry(page_key).or_default();
        page.total = total;
        page.next_cursor = next_cursor;
        if append {
            page.items.extend(items);
        } else {
            page.items = items;
        }
    }

    // 丢弃当前 List 查询结果并从首屏重新拉取，用于「刷新」按钮等需要强制回源的场景。
    fn rerun_redis_list_item_search(&mut self, tab_id: TabId, key: String, cx: &mut Context<Self>) {
        self.redis_list_item_search_generation
            .entry(tab_id.0)
            .and_modify(|generation| *generation += 1)
            .or_insert(0);
        self._data_load_tasks
            .remove(&redis_list_item_search_task_id(tab_id));
        self.redis_list_item_search_loading = None;
        self.redis_list_item_search_more_loading = None;
        let query = self
            .redis_list_item_search_queries
            .get(&(tab_id, key.clone()))
            .cloned()
            .unwrap_or_default();
        self.request_redis_list_item_search(tab_id, key, query, String::new(), cx);
    }

    fn start_redis_list_item_mutation(
        &mut self,
        tab_id: TabId,
        key: String,
        commands: Vec<AppCommand>,
        cx: &mut Context<Self>,
    ) {
        let task_id = redis_list_item_mutation_task_id(tab_id);
        if self._data_load_tasks.contains_key(&task_id) {
            return;
        }
        self.redis_list_item_search_generation
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
                                    message: "List 元素操作没有返回结果".to_string(),
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
                        // 变更成功后用当前搜索词重拉首屏
                        Ok(()) => this.rerun_redis_list_item_search(
                            tab_id,
                            key.clone(),
                            cx,
                        ),
                        Err(error) => this.show_message(error.message, AppMessageKind::Error, cx),
                    }
                    cx.notify();
                });
            });
        });
        self._data_load_tasks.insert(task_id, task);
        cx.notify();
    }

    fn active_redis_list_page(&self, tab_id: TabId, key: &str) -> Option<RedisListItemPage> {
        let query = self
            .redis_list_item_search_queries
            .get(&(tab_id, key.to_string()))
            .cloned()
            .unwrap_or_default();
        self.redis_list_item_search_pages
            .get(&(tab_id, key.to_string(), query))
            .cloned()
    }

    fn sync_redis_list_item_inputs(
        &mut self,
        tab_id: TabId,
        detail: &RedisKeyDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = (tab_id, detail.key.clone());
        let expected = self
            .redis_list_item_search_queries
            .get(&active)
            .cloned()
            .unwrap_or_default();
        let active_changed = self.redis_list_item_search_active.as_ref() != Some(&active);
        // 切换 key/标签页后清理遗留的行内编辑与 hover 状态，避免编辑浮层或高亮残留在错误 key 上
        if active_changed {
            self.redis_list_item_editing = None;
            self.redis_list_item_hovered = None;
        }
        // 切换 key：作废上一 key 的在飞搜索并按新 key 重发首屏（同 Set/Hash/ZSet 口径）。
        if active_changed {
            self.rerun_redis_list_item_search(tab_id, detail.key.clone(), cx);
        }
        let focused = self
            .redis_list_item_search_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let current = self.redis_list_item_search_input.read(cx).value().to_string();
        if active_changed || (!focused && current != expected) {
            self.redis_list_item_search_syncing = true;
            self.redis_list_item_search_input.update(cx, |input, cx| {
                input.set_value(expected, window, cx);
            });
            self.redis_list_item_search_syncing = false;
        }
        // 记录已同步的 (tab, key)，避免每帧重复触发 LRANGE 查询。
        self.redis_list_item_search_active = Some(active);
    }
}
