impl NavicatMain {
    fn redis_zset_member_rows_snapshot(&self, cx: &App) -> Vec<(String, String)> {
        self.redis_zset_member_rows
            .iter()
            .map(|row| {
                (
                    row.member_input.read(cx).value().to_string(),
                    row.score_input.read(cx).value().to_string(),
                )
            })
            .collect()
    }

    fn redis_zset_member_drawer_rows_snapshot(&self, cx: &App) -> Vec<(String, String)> {
        self.redis_zset_member_drawer_rows
            .iter()
            .map(|row| {
                (
                    row.member_input.read(cx).value().to_string(),
                    row.score_input.read(cx).value().to_string(),
                )
            })
            .collect()
    }

    fn open_redis_zset_member_add_drawer(
        &mut self,
        tab_id: TabId,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_redis_zset_member_delete = None;
        self.pending_redis_zset_member_drawer = Some(RedisZSetMemberDrawerForm { tab_id, key });
        self.redis_zset_member_drawer_rows = vec![RedisZSetMemberInputs {
            member_input: new_redis_zset_member_input(window, cx, None),
            score_input: new_redis_zset_score_input(window, cx, None),
        }];
        self.redis_zset_member_drawer_scroll.set_offset(Point::default());
        cx.notify();
    }

    fn add_redis_zset_member_drawer_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.redis_zset_member_drawer_rows.push(RedisZSetMemberInputs {
            member_input: new_redis_zset_member_input(window, cx, None),
            score_input: new_redis_zset_score_input(window, cx, None),
        });
        self.redis_zset_member_drawer_scroll.scroll_to_bottom();
        if let Some(row) = self.redis_zset_member_drawer_rows.last() {
            row.member_input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn remove_redis_zset_member_drawer_row(
        &mut self,
        row_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if row_index >= self.redis_zset_member_drawer_rows.len() {
            return;
        }
        self.redis_zset_member_drawer_rows.remove(row_index);
        let focus_index = row_index.min(self.redis_zset_member_drawer_rows.len().saturating_sub(1));
        if let Some(row) = self.redis_zset_member_drawer_rows.get(focus_index) {
            row.member_input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn cancel_redis_zset_member_drawer(&mut self, cx: &mut Context<Self>) {
        self.pending_redis_zset_member_drawer = None;
        self.redis_zset_member_drawer_rows.clear();
        self.pending_redis_zset_member_delete = None;
        cx.notify();
    }

    fn confirm_redis_zset_member_drawer(
        &mut self,
        tab_id: TabId,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rows = self
            .redis_zset_member_drawer_rows_snapshot(cx)
            .into_iter()
            .map(|(member, score)| (member.trim().to_string(), score.trim().to_string()))
            .collect::<Vec<_>>();
        if rows.is_empty() {
            self.show_message("至少输入一个成员", AppMessageKind::Warning, cx);
            return;
        }
        if rows.iter().any(|(member, _)| member.is_empty()) {
            self.show_message("成员不能为空", AppMessageKind::Warning, cx);
            return;
        }
        self.pending_redis_zset_member_drawer = None;
        self.redis_zset_member_drawer_rows.clear();
        self.pending_redis_zset_member_delete = None;
        let commands = rows
            .into_iter()
            .map(|(member, score)| AppCommand::AddRedisZSetMember {
                tab_id,
                key: key.clone(),
                member,
                score,
            })
            .collect::<Vec<_>>();
        self.start_redis_zset_member_mutation(tab_id, key, commands, cx);
        let _ = window;
    }

    fn request_redis_zset_member_search(
        &mut self,
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
        cx: &mut Context<Self>,
    ) {
        let task_id = redis_zset_member_search_task_id(tab_id);
        if self._data_load_tasks.contains_key(&task_id) {
            return;
        }
        let is_more = !cursor.is_empty();
        if is_more
            && self.redis_zset_member_search_more_loading.as_ref()
                == Some(&(tab_id, key.clone(), query.clone()))
        {
            return;
        }
        self.redis_zset_member_search_more_loading = None;
        let generation = *self
            .redis_zset_member_search_generation
            .entry(tab_id.0)
            .or_default();
        let mut controller = self.controller.clone();
        let req_key = key.clone();
        let req_query = query.clone();
        let req_cursor = cursor.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadRedisZSetMembers {
                        tab_id,
                        key: req_key.clone(),
                        query: req_query.clone(),
                        cursor: req_cursor.clone(),
                    }) {
                        AppEvent::RedisZSetMembersLoaded {
                            key,
                            query,
                            cursor,
                            members,
                            next_cursor,
                            total,
                            ..
                        } => Ok((key, query, cursor, members, next_cursor, total)),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "查询失败".to_string(),
                            message: "ZSet 成员查询没有返回结果".to_string(),
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
                    this.redis_zset_member_search_loading = None;
                    this.redis_zset_member_search_more_loading = None;
                    let still_current = this
                        .redis_zset_member_search_generation
                        .get(&tab_id.0)
                        .copied()
                        .unwrap_or_default()
                        == generation;
                    if !still_current {
                        return;
                    }
                    match result {
                        Ok((key, query, cursor, members, next_cursor, total)) => {
                            this.store_redis_zset_member_search_page(
                                tab_id,
                                &key,
                                &query,
                                members,
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
        self.redis_zset_member_search_loading = Some((tab_id, key.clone(), query.clone()));
        if is_more {
            self.redis_zset_member_search_more_loading = Some((tab_id, key.clone(), query.clone()));
        }
        self._data_load_tasks.insert(task_id, task);
        cx.notify();
    }

    fn store_redis_zset_member_search_page(
        &mut self,
        tab_id: TabId,
        key: &str,
        query: &str,
        members: Vec<(String, String)>,
        next_cursor: String,
        total: usize,
        append: bool,
    ) {
        let page_key = (tab_id, key.to_string(), query.to_string());
        let page = self.redis_zset_member_search_pages.entry(page_key).or_default();
        page.total = total;
        page.next_cursor = next_cursor;
        if append {
            let existing = page.members.clone().into_iter().collect::<BTreeSet<_>>();
            for member in members {
                if !existing.contains(&member) {
                    page.members.push(member);
                }
            }
        } else {
            page.members = members;
        }
    }

    fn schedule_redis_zset_member_search(
        &mut self,
        tab_id: TabId,
        key: String,
        query: String,
        cx: &mut Context<Self>,
    ) {
        let until = Instant::now() + Duration::from_millis(350);
        self.redis_zset_member_search_debounce_until = Some(until);
        let req_tab_id = tab_id;
        let req_key = key.clone();
        let req_query = query.clone();
        self.redis_zset_member_search_debounce = Some(cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(350))
                .await;
            view.update(cx, move |this, cx| {
                let still_pending = this
                    .redis_zset_member_search_debounce_until
                    .is_some_and(|current_until| current_until == until);
                if still_pending {
                    this.redis_zset_member_search_debounce_until = None;
                    this.redis_zset_member_search_debounce = None;
                    // 防抖在「输入时」就抓了 key；350ms 后用户可能已切到别的键，
                    // 按旧键发搜索会对不再展示的旧键误发（如跨 key 的 WRONGTYPE 误报）。
                    // 校验当前 panel 的 active key 是否仍是捕获时的键，不是则丢弃。
                    let on_active_key = this.redis_zset_member_search_active.as_ref()
                        == Some(&(req_tab_id, req_key.clone()));
                    if on_active_key {
                        this.request_redis_zset_member_search(
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

    fn rerun_redis_zset_member_search(&mut self, tab_id: TabId, key: String, cx: &mut Context<Self>) {
        let query = self
            .redis_zset_member_search_queries
            .get(&(tab_id, key.clone()))
            .cloned()
            .unwrap_or_default();
        self.redis_zset_member_search_generation
            .entry(tab_id.0)
            .and_modify(|generation| *generation += 1)
            .or_insert(0);
        self.redis_zset_member_search_debounce = None;
        self._data_load_tasks
            .remove(&redis_zset_member_search_task_id(tab_id));
        self.redis_zset_member_search_loading = None;
        self.redis_zset_member_search_more_loading = None;
        self.request_redis_zset_member_search(tab_id, key, query, String::new(), cx);
    }

    fn request_redis_zset_member_delete(
        &mut self,
        tab_id: TabId,
        key: String,
        member: String,
        cx: &mut Context<Self>,
    ) {
        self.start_redis_zset_member_mutation(
            tab_id,
            key.clone(),
            vec![AppCommand::DeleteRedisZSetMember {
                tab_id,
                key,
                member,
            }],
            cx,
        );
    }

    fn start_redis_zset_member_mutation(
        &mut self,
        tab_id: TabId,
        key: String,
        commands: Vec<AppCommand>,
        cx: &mut Context<Self>,
    ) {
        let task_id = redis_zset_member_mutation_task_id(tab_id);
        if self._data_load_tasks.contains_key(&task_id) {
            return;
        }
        self.redis_zset_member_search_generation
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
                                    message: "ZSet 成员操作没有返回结果".to_string(),
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
                        Ok(()) => this.rerun_redis_zset_member_search(tab_id, key, cx),
                        Err(error) => this.show_message(error.message, AppMessageKind::Error, cx),
                    }
                    cx.notify();
                });
            });
        });
        self._data_load_tasks.insert(task_id, task);
        cx.notify();
    }

    fn active_redis_zset_page(&self, tab_id: TabId, key: &str) -> Option<RedisZSetMemberPage> {
        let query = self
            .redis_zset_member_search_queries
            .get(&(tab_id, key.to_string()))
            .cloned()
            .unwrap_or_default();
        self.redis_zset_member_search_pages
            .get(&(tab_id, key.to_string(), query))
            .cloned()
    }

    fn sync_redis_zset_member_inputs(
        &mut self,
        tab_id: TabId,
        detail: &RedisKeyDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = (tab_id, detail.key.clone());
        let expected = self
            .redis_zset_member_search_queries
            .get(&active)
            .cloned()
            .unwrap_or_default();
        let active_changed = self.redis_zset_member_search_active.as_ref() != Some(&active);
        // 切换 key：作废上一 key 的在飞搜索并按新 key 重发首屏（同 Set/Hash 口径）。
        if active_changed {
            self.rerun_redis_zset_member_search(tab_id, detail.key.clone(), cx);
        }
        let focused = self
            .redis_zset_member_search_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let current = self.redis_zset_member_search_input.read(cx).value().to_string();
        if active_changed || (!focused && current != expected) {
            self.redis_zset_member_search_syncing = true;
            self.redis_zset_member_search_input.update(cx, |input, cx| {
                input.set_value(expected, window, cx);
            });
            self.redis_zset_member_search_syncing = false;
        }
        if let Some(page) = self.active_redis_zset_page(tab_id, &detail.key) {
            let rows = page.members.clone();
            if self.redis_zset_member_rows.len() != rows.len()
                || self.redis_zset_member_rows_snapshot(cx) != rows
            {
                self.redis_zset_member_rows = rows
                    .into_iter()
                    .map(|(member, score)| RedisZSetMemberInputs {
                        member_input: new_redis_zset_member_input(window, cx, Some(member)),
                        score_input: new_redis_zset_score_input(window, cx, Some(score)),
                    })
                    .collect();
                // 行集合重建（翻页/刷新/保存后）：清除失效的 hover、行内编辑态与删除确认态，
                // 避免编辑框/确认浮层悬空指向已不存在的行。
                self.redis_zset_member_score_hover = None;
                self.cancel_redis_zset_member_score_edit(cx);
                self.pending_redis_zset_member_delete = None;
            }
        }
        // 记录已同步的 (tab, key)，避免每帧重复触发 ZSCAN 查询。
        self.redis_zset_member_search_active = Some(active);
    }

    /// 进入 score 单元格行内编辑：以当前行的 score 作为初始值创建独立编辑输入并聚焦。
    /// 同时为编辑输入注册失焦回调：点击输入框以外（另一个输入、搜索框、按钮等可聚焦区域）
    /// 导致焦点丢失时自动取消编辑，对齐 RedisInsight 的「点击其他地方取消编辑」。
    fn begin_redis_zset_member_score_edit(
        &mut self,
        row_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 切换编辑行（点击另一行的编辑图标）时，旧输入的失焦回调会在下方注册的新编辑
        // 生效后再触发；用 captured_id 做守卫，仅当失焦的仍是当前活跃编辑输入时才取消，
        // 避免刚切过去就被旧输入的 blur 冲掉。
        let Some(row) = self.redis_zset_member_rows.get(row_index) else {
            return;
        };
        let current = row.score_input.read(cx).value().to_string();
        let edit_input = new_redis_zset_score_input(window, cx, Some(current));
        let captured_id = edit_input.entity_id();
        let blur_handle = edit_input.read(cx).focus_handle(cx).clone();
        let blur_sub = cx.on_blur(
            &blur_handle,
            window,
            move |this: &mut NavicatMain, _, cx| {
                if this
                    .redis_zset_member_score_edit_input
                    .as_ref()
                    .is_some_and(|input| input.entity_id() == captured_id)
                {
                    this.cancel_redis_zset_member_score_edit(cx);
                }
            },
        );
        // 保存失焦订阅，避免被立即注销导致点击外部无法取消。
        self.redis_zset_member_score_blur_sub = Some(blur_sub);
        edit_input.update(cx, |input, cx| input.focus(window, cx));
        self.redis_zset_member_score_edit_input = Some(edit_input);
        self.redis_zset_member_score_editing = Some(row_index);
        cx.notify();
    }

    /// 取消行内编辑：直接丢弃编辑输入，不写回任何值。
    fn cancel_redis_zset_member_score_edit(&mut self, cx: &mut Context<Self>) -> bool {
        if self.redis_zset_member_score_editing.take().is_none()
            && self.redis_zset_member_score_edit_input.is_none()
        {
            return false;
        }
        self.redis_zset_member_score_editing = None;
        self.redis_zset_member_score_edit_input = None;
        self.redis_zset_member_score_blur_sub = None;
        cx.notify();
        true
    }

    /// 确认行内编辑：校验 score 为数字，写回对应行 score 输入，并**立即落库**修改该 member 的 score。
    /// 由于底部「保存」按钮已移除，score 的变更就靠这里的「勾」直接提交 `UpdateRedisZSetScore`。
    fn apply_redis_zset_member_score_edit(
        &mut self,
        tab_id: TabId,
        key: String,
        row_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(editing) = self.redis_zset_member_score_editing else {
            return false;
        };
        if editing != row_index {
            return false;
        }
        let Some(edit_input) = self.redis_zset_member_score_edit_input.clone() else {
            return false;
        };
        let current = edit_input.read(cx).value().trim().to_string();
        // 对齐 Redis ZADD 对 score 的约束：必须是合法数字（整数或双精度浮点）。
        if current.parse::<f64>().is_err() {
            self.show_message("Score 必须是数字", AppMessageKind::Warning, cx);
            return false;
        }
        let Some(row) = self.redis_zset_member_rows.get(row_index).cloned() else {
            self.cancel_redis_zset_member_score_edit(cx);
            return false;
        };
        row.score_input
            .update(cx, |input, cx| input.set_value(current.clone(), window, cx));
        let member = row.member_input.read(cx).value().to_string();
        self.redis_zset_member_score_editing = None;
        self.redis_zset_member_score_edit_input = None;
        self.redis_zset_member_score_blur_sub = None;
        self.redis_zset_member_score_hover = None;
        cx.notify();
        // 立即提交：与当前页缓存（上次 ZSCAN 的 old_score）对比，score 有变化才发 ZADD。
        let changed = self
            .active_redis_zset_page(tab_id, &key)
            .and_then(|page| page.members.get(row_index).cloned())
            .is_some_and(|(old_member, old_score)| old_member == member && old_score != current);
        if changed {
            self.start_redis_zset_member_mutation(
                tab_id,
                key.clone(),
                vec![AppCommand::UpdateRedisZSetScore {
                    tab_id,
                    key,
                    member,
                    score: current,
                }],
                cx,
            );
        }
        true
    }
}
