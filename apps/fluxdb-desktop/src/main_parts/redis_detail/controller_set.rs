impl NavicatMain {
    // 发起 Set 成员服务端分页搜索：cursor 为 "" 表示替换首屏，非 "" 表示追加「加载更多」。
    // 同一个 tab 同时只允许一个在飞搜索（_data_load_tasks 去重），结果只写入 search_pages，
    // 行 entity 由 sync_redis_set_member_inputs 在渲染期重建（重建需要 Window，异步闭包拿不到）。
    fn request_redis_set_member_search(
        &mut self,
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
        cx: &mut Context<Self>,
    ) {
        let task_id = redis_set_member_search_task_id(tab_id);
        if self._data_load_tasks.contains_key(&task_id) {
            return;
        }
        let is_more = !cursor.is_empty();
        if is_more
            && self.redis_set_member_search_more_loading.as_ref()
                == Some(&(tab_id, key.clone(), query.clone()))
        {
            return;
        }
        self.redis_set_member_search_more_loading = None;
        // 代际计数：增删期间发起的新搜索会让在飞旧搜索过期，完成时丢弃不覆盖
        let generation = *self
            .redis_set_member_search_generation
            .entry(tab_id.0)
            .or_default();
        let mut controller = self.controller.clone();
        let req_key = key.clone();
        // 用于 WRONGTYPE 后触发键详情重探测的独立克隆，避免与 req_key（被在飞查询拿走）冲突。
        let recovery_key = key.clone();
        let req_query = query.clone();
        let req_cursor = cursor.clone();
        let task = cx.spawn(async move |view, cx| {
            // 保留一份 req_key 用于结果回填时的「当前键」校验；req_key 会被内层后台任务 move 走。
            let still_current_key = req_key.clone();
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadRedisSetMembers {
                        tab_id,
                        key: req_key.clone(),
                        query: req_query.clone(),
                        cursor: req_cursor.clone(),
                    }) {
                        AppEvent::RedisSetMembersLoaded {
                            tab_id: _,
                            key,
                            query,
                            cursor,
                            members,
                            next_cursor,
                            total,
                        } => Ok((key, query, cursor, members, next_cursor, total)),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "查询失败".to_string(),
                            message: "Set 成员查询没有返回结果".to_string(),
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
                    this.redis_set_member_search_loading = None;
                    this.redis_set_member_search_more_loading = None;
                    // 代际不一致说明此搜索已过期（增删/新搜索期间被替换），丢弃结果不覆盖；
                    // 搜索的 key 不再是当前 panel 的 key（用户已切走/残留事件误发）同样丢弃，
                    // 否则会出现「点 bigset 却弹 hash_key 不是 Set」这类跨 key 误报。
                    let still_current = this
                        .redis_set_member_search_generation
                        .get(&tab_id.0)
                        .copied()
                        .unwrap_or_default()
                        == generation
                        && this.redis_set_member_search_active.as_ref()
                            == Some(&(tab_id, still_current_key.clone()));
                    if !still_current {
                        return;
                    }
                    match result {
                        Ok((key, query, cursor, members, next_cursor, total)) => {
                            this.store_redis_set_member_search_page(
                                tab_id,
                                &key,
                                &query,
                                members,
                                next_cursor,
                                total,
                                !cursor.is_empty(),
                            );
                            // 在飞期间查询已变化：补发最新查询的首屏搜索。
                            // 旧页留给旧查询键，不影响 UI（渲染按最新 query 取页）。
                            let latest_query = this
                                .redis_set_member_search_queries
                                .get(&(tab_id, key.clone()))
                                .cloned()
                                .unwrap_or_default();
                            if latest_query != query {
                                this.request_redis_set_member_search(
                                    tab_id, key, latest_query, String::new(), cx,
                                );
                            }
                        }
                        Err(error) => {
                            this.show_message(error.message.clone(), AppMessageKind::Error, cx);
                            // WRONGTYPE 说明该键在服务端已不是 Set（常因被删除后重建为其他类型、
                            // 或密钥列表里缓存的类型已过期）。只弹提示会一直卡在错误的 Set 面板上，
                            // 所以额外触发一次键详情重探测，让面板切回真实类型。
                            if error.message.contains("不是 Set") {
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
        self.redis_set_member_search_loading = Some((tab_id, key.clone(), query.clone()));
        if is_more {
            self.redis_set_member_search_more_loading = Some((tab_id, key.clone(), query.clone()));
        }
        self._data_load_tasks.insert(task_id, task);
        cx.notify();
    }

    // 存储一页搜索结果：append=true 去重追加 members，append=false 整体替换。
    // 不重建行 entity，交给渲染期 sync_redis_set_member_inputs。
    fn store_redis_set_member_search_page(
        &mut self,
        tab_id: TabId,
        key: &str,
        query: &str,
        members: Vec<String>,
        next_cursor: String,
        total: usize,
        append: bool,
    ) {
        let page_key = (tab_id, key.to_string(), query.to_string());
        let page = self
            .redis_set_member_search_pages
            .entry(page_key)
            .or_default();
        page.total = total;
        page.next_cursor = next_cursor;
        if append {
            // SSCAN 分 bucket/resharding 可能重复，追加时按成员字符串去重
            let existing = page.members.iter().cloned().collect::<BTreeSet<_>>();
            for member in members {
                if !existing.contains(&member) {
                    page.members.push(member);
                }
            }
        } else {
            page.members = members;
        }
    }

    // 防抖发起排除式搜索：350ms 内连续输入只执行最后一次。
    // 用 debounce_until 令牌保证旧触发不会覆盖新查询（替换在飞 Task 同样可取消）。
    fn schedule_redis_set_member_search(
        &mut self,
        tab_id: TabId,
        key: String,
        query: String,
        cx: &mut Context<Self>,
    ) {
        let until = Instant::now() + Duration::from_millis(350);
        self.redis_set_member_search_debounce_until = Some(until);
        let req_tab_id = tab_id;
        let req_key = key.clone();
        let req_query = query.clone();
        self.redis_set_member_search_debounce = Some(cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(350))
                .await;
            view.update(cx, move |this, cx| {
                let still_pending = this
                    .redis_set_member_search_debounce_until
                    .is_some_and(|current_until| current_until == until);
                if still_pending {
                    this.redis_set_member_search_debounce_until = None;
                    this.redis_set_member_search_debounce = None;
                    // 防抖在「输入时」就抓了 key；350ms 后用户可能已切到别的键，
                    // 此时按旧键发首屏搜索会对不再展示的旧键触发 WRONGTYPE。
                    // 校验当前 panel 的 active key 是否仍是捕获时的键，不是则丢弃。
                    let on_active_key = this.redis_set_member_search_active.as_ref()
                        == Some(&(req_tab_id, req_key.clone()));
                    if on_active_key {
                        this.request_redis_set_member_search(
                            req_tab_id, req_key, req_query, String::new(), cx,
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    // 重跑当前搜索（cursor ""）：增删成员后需重新分页，让总数/命中数收敛到服务端真实值。
    // bump generation 使在飞旧搜索过期（其完成时不覆盖），并取消在飞去重强制重发。
    fn rerun_redis_set_member_search(&mut self, tab_id: TabId, key: String, cx: &mut Context<Self>) {
        let query = self
            .redis_set_member_search_queries
            .get(&(tab_id, key.clone()))
            .cloned()
            .unwrap_or_default();
        self.redis_set_member_search_generation
            .entry(tab_id.0)
            .and_modify(|generation| *generation += 1)
            .or_insert(0);
        self.redis_set_member_search_debounce = None;
        // 替换在飞搜索 Task 即取消它（gpui 持有 Task 可取消在飞 future）
        self._data_load_tasks
            .remove(&redis_set_member_search_task_id(tab_id));
        self.redis_set_member_search_loading = None;
        self.redis_set_member_search_more_loading = None;
        self.request_redis_set_member_search(tab_id, key, query, String::new(), cx);
    }

    // per-member 新增（SADD）：逐个成员发命令，完成后重跑当前搜索刷新分页。
    // 不复用整集合重写，避免分页下把未加载成员当作删除。
    fn request_redis_set_member_add(
        &mut self,
        tab_id: TabId,
        key: String,
        members: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if members.is_empty() {
            return;
        }
        let commands = members
            .into_iter()
            .map(|member| AppCommand::AddRedisSetMember {
                tab_id,
                key: key.clone(),
                member,
            })
            .collect::<Vec<_>>();
        self.start_redis_set_member_mutation(tab_id, key, commands, cx);
    }

    // per-member 删除（SREM）：只删指定成员，完成后重跑当前搜索刷新分页。
    fn request_redis_set_member_delete(
        &mut self,
        tab_id: TabId,
        key: String,
        member: String,
        cx: &mut Context<Self>,
    ) {
        self.start_redis_set_member_mutation(
            tab_id,
            key.clone(),
            vec![AppCommand::DeleteRedisSetMember {
                tab_id,
                key,
                member,
            }],
            cx,
        );
    }

    // Set 成员增删命令的异步执行：批内命令串行 dispatch（与搜索任务用不同 task_id，
    // 互不阻塞）。完成后强刷当前搜索（bump generation，使在飞旧搜索过期并强制重发）。
    fn start_redis_set_member_mutation(
        &mut self,
        tab_id: TabId,
        key: String,
        commands: Vec<AppCommand>,
        cx: &mut Context<Self>,
    ) {
        let task_id = redis_set_member_mutation_task_id(tab_id);
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
                                    message: "Set 成员操作没有返回结果".to_string(),
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
                            this.rerun_redis_set_member_search(tab_id, key, cx);
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

    // 打开 Set「新增成员」底部抽屉：重置抽屉行并聚焦第一个输入框
    fn open_redis_set_member_add_drawer(
        &mut self,
        tab_id: TabId,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_redis_set_member_delete = None;
        self.pending_redis_set_member_drawer = Some(RedisSetMemberDrawerForm { tab_id, key });
        self.redis_set_member_drawer_rows = vec![new_redis_set_member_input(window, cx, None)];
        self.redis_set_member_drawer_scroll.scroll_to_bottom();
        if let Some(input) = self.redis_set_member_drawer_rows.first() {
            input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        }
        cx.notify();
    }

    // 抽屉内新增一行输入，并自动滚动到底部露出新行
    fn add_redis_set_member_drawer_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.redis_set_member_drawer_rows
            .push(new_redis_set_member_input(window, cx, None));
        // 达到最大高度后自动滚动到底部，保证新输入框可见
        self.redis_set_member_drawer_scroll.scroll_to_bottom();
        if let Some(input) = self.redis_set_member_drawer_rows.last() {
            input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        }
        cx.notify();
    }

    // 删除抽屉内指定行，焦点回退到相邻行
    fn remove_redis_set_member_drawer_row(
        &mut self,
        row_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if row_index >= self.redis_set_member_drawer_rows.len() {
            return;
        }

        self.redis_set_member_drawer_rows.remove(row_index);
        let focus_index = row_index.min(self.redis_set_member_drawer_rows.len().saturating_sub(1));
        if let Some(input) = self.redis_set_member_drawer_rows.get(focus_index) {
            input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        }
        cx.notify();
    }

    // 关闭抽屉并丢弃未保存的输入
    fn cancel_redis_set_member_drawer(&mut self, cx: &mut Context<Self>) {
        self.pending_redis_set_member_drawer = None;
        self.redis_set_member_drawer_rows.clear();
        self.pending_redis_set_member_delete = None;
        cx.notify();
    }

    // 保存抽屉输入：只对抽屉里输入的成员做 per-member SADD（新增），
    // 不复用整集合重写（分页下重写会误删未加载成员），成功后由搜索重跑刷新面板。
    fn confirm_redis_set_member_drawer(
        &mut self,
        tab_id: TabId,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let values = self
            .redis_set_member_drawer_rows_snapshot(cx)
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>();
        if values.is_empty() {
            self.show_message("至少输入一个 Member", AppMessageKind::Warning, cx);
            return;
        }
        self.pending_redis_set_member_drawer = None;
        self.redis_set_member_drawer_rows.clear();
        self.pending_redis_set_member_delete = None;
        // SADD：集合天然去重，重复输入再写无副作用
        self.request_redis_set_member_add(
            tab_id,
            key.clone(),
            values.into_iter().collect::<Vec<_>>(),
            cx,
        );
        let _ = window;
        cx.notify();
    }

    // 面板「取消」：从服务端数据重建成员输入行，丢弃本地编辑
    fn discard_redis_set_member_rows(
        &mut self,
        tab_id: TabId,
        detail: &RedisKeyDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (_, members) = redis_set_preview_members(&detail.value);
        self.redis_set_member_rows = members
            .into_iter()
            .map(|member| new_redis_set_member_input(window, cx, Some(member)))
            .collect();
        self.redis_set_member_active = Some((tab_id, detail.key.clone()));
        cx.notify();
    }

    fn sync_redis_set_member_inputs(
        &mut self,
        tab_id: TabId,
        detail: &RedisKeyDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = (tab_id, detail.key.clone());
        let active_changed = self.redis_set_member_active.as_ref() != Some(&active);
        // 当前提交过的搜索词（与搜索框同步逻辑一致）
        let current_query = self
            .redis_set_member_search_queries
            .get(&active)
            .cloned()
            .unwrap_or_default();
        // 服务端搜索结果存在时，面板行以搜索页为准，不再用截断 preview 重建；
        // 删除确认浮层打开期间不重建：避免 sync 误清 pending 导致浮层消失、行索引变化误删。
        let has_pending_delete = self.pending_redis_set_member_delete.is_some();
        if !has_pending_delete {
            let server_page = self
                .redis_set_member_search_pages
                .get(&(tab_id, detail.key.clone(), current_query.clone()))
                .cloned()
                .or_else(|| {
                    (current_query.is_empty())
                        .then(|| {
                            self.redis_set_member_search_pages
                                .get(&(tab_id, detail.key.clone(), String::new()))
                                .cloned()
                        })
                        .flatten()
                });
            if let Some(page) = server_page {
                // 值变化才重建 entity，避免每帧重建造成输入框焦点抖动
                if self.redis_set_member_rows_snapshot(cx) != page.members {
                    self.redis_set_member_rows = page
                        .members
                        .iter()
                        .map(|member| new_redis_set_member_input(window, cx, Some(member.clone())))
                        .collect();
                }
                self.redis_set_member_active = Some(active);
                return;
            }
        }
        // 无服务端结果（首次打开搜索在飞）：维持 preview 重建兜底
        let (_, members) = redis_set_preview_members(&detail.value);
        // 面板行为只读展示、由抽屉合并后重建，值可能领先服务端（落库异步窗口），
        // 因此仅当与服务端 member 一致时重建 entity（无害），不一致时保留本地合并态，
        // 避免把抽屉保存的新成员覆盖回旧 server 值。
        if active_changed {
            // 切换 key：无条件从服务端重建
            self.pending_redis_set_member_delete = None;
            self.redis_set_member_rows = members
                .into_iter()
                .map(|member| new_redis_set_member_input(window, cx, Some(member)))
                .collect();
        } else if !has_pending_delete && self.redis_set_member_rows_snapshot(cx) == members {
            // 值一致时重建（替换 entity，刷新内部状态），不影响用户可见内容
            self.redis_set_member_rows = members
                .into_iter()
                .map(|member| new_redis_set_member_input(window, cx, Some(member)))
                .collect();
        }
        self.redis_set_member_active = Some(active);
    }

    fn sync_redis_set_member_search_input(
        &mut self,
        tab_id: TabId,
        detail: &RedisKeyDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !detail.kind.eq_ignore_ascii_case("set") {
            return;
        }
        let active = (tab_id, detail.key.clone());
        let expected = self
            .redis_set_member_search_queries
            .get(&active)
            .cloned()
            .unwrap_or_default();
        let active_changed = self.redis_set_member_search_active.as_ref() != Some(&active);
        // 切换 key：无条件作废上一 key 的在飞搜索（取消 Task + bump 代际）并按新 key 重发首屏，
        // 让面板脱离截断 preview、走服务端分页。不能用 `active_changed && !在飞` 判定：
        // 搜索任务 id 按 tab 去重，快速连续点击不同 set key 时新 key 的搜索会被旧 key 的在飞任务
        // 拦下，而旧 key 的 WRONGTYPE/失败结果却会回显，弹成「键「旧key」不是 Set」的错误。
        if active_changed {
            self.rerun_redis_set_member_search(tab_id, detail.key.clone(), cx);
        }
        let focused = self
            .redis_set_member_search_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let current = self.redis_set_member_search_input.read(cx).value().to_string();
        if active_changed || (!focused && current != expected) {
            self.redis_set_member_search_syncing = true;
            self.redis_set_member_search_input.update(cx, |input, cx| {
                input.set_value(expected, window, cx);
            });
            self.redis_set_member_search_syncing = false;
        }
        self.redis_set_member_search_active = Some(active);
    }

    fn redis_set_member_rows_snapshot(&self, cx: &App) -> Vec<String> {
        self.redis_set_member_rows
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect()
    }

    // 抽屉内输入框当前值快照
    fn redis_set_member_drawer_rows_snapshot(&self, cx: &App) -> Vec<String> {
        self.redis_set_member_drawer_rows
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect()
    }
}
