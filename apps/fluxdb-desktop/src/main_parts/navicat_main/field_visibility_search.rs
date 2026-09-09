impl NavicatMain {
    fn refresh_active_data_table(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(page) = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => Some(editor),
                _ => None,
            })
            .and_then(|editor| editor.page.clone())
        {
            self.refresh_data_table_state(tab_id, &page, cx);
            cx.notify();
            return;
        }

        let query_result = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::QueryEditor(editor) => {
                    let result_index = self
                        .query_output_tabs
                        .get(&tab_id)
                        .and_then(|tab| tab.result_index())
                        .unwrap_or(0);
                    let page_index = query_result_page_index(editor, result_index)?;
                    let result_editor = editor.result_editors.get(&page_index).cloned();
                    let page = result_editor
                        .as_ref()
                        .and_then(|editor| editor.page.clone())
                        .or_else(|| editor.results.get(page_index).cloned())?;
                    Some((result_index, page_index, page, result_editor))
                }
                _ => None,
            });
        if let Some((result_index, page_index, page, result_editor)) = query_result {
            self.refresh_query_result_table_state(
                tab_id,
                &page,
                page_index,
                result_index,
                result_editor.as_ref(),
                cx,
            );
        }
        cx.notify();
    }

    fn update_field_filter_search(&mut self, query: String, cx: &mut Context<Self>) {
        self.field_filter_search = query;
        cx.notify();
    }

    fn active_redis_data_tab_id(&self) -> Option<TabId> {
        let tab = self.controller.state().active_tab()?;
        matches!(
            &tab.kind,
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey)
        )
        .then_some(tab.id)
    }

    fn data_page_for_display<'a>(&self, tab_id: TabId, page: &'a DataPage) -> Cow<'a, DataPage> {
        if !self.is_redis_data_tab(tab_id) {
            return Cow::Borrowed(page);
        }
        let type_filter = self.redis_type_filter(tab_id);
        let query = self.redis_search_query(tab_id);
        if type_filter == REDIS_TYPE_FILTER_ALL && query.is_empty() {
            return Cow::Borrowed(page);
        }
        Cow::Owned(redis_filter_page(page, type_filter, query))
    }

    fn is_redis_data_tab(&self, tab_id: TabId) -> bool {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .is_some_and(|tab| {
                matches!(
                    &tab.kind,
                    TabKind::DataEditor(editor)
                        if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey)
                )
            })
    }

    fn redis_search_query(&self, tab_id: TabId) -> &str {
        self.redis_search_queries
            .get(&tab_id)
            .map(String::as_str)
            .unwrap_or_default()
    }

    fn redis_search_draft(&self, tab_id: TabId) -> &str {
        self.redis_search_drafts
            .get(&tab_id)
            .map(String::as_str)
            .unwrap_or_default()
    }

    fn redis_type_filter(&self, tab_id: TabId) -> &str {
        self.redis_type_filters
            .get(&tab_id)
            .map(String::as_str)
            .unwrap_or(REDIS_TYPE_FILTER_ALL)
    }

    fn apply_redis_search(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let query = self.redis_search_draft(tab_id).trim().to_string();
        self.redis_key_folder_visible_cache.remove(&tab_id);
        if query.is_empty() {
            self.redis_search_queries.remove(&tab_id);
        } else {
            self.redis_search_queries.insert(tab_id, query.clone());
            // 一次成功应用即作为一条搜索历史（按连接 + DB 隔离），并落盘。
            self.record_redis_key_search_history(tab_id);
        }
        self.refresh_active_data_table(tab_id, cx);
        cx.notify();
    }

    fn clear_redis_search(&mut self, tab_id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        self.redis_search_drafts.remove(&tab_id);
        self.redis_search_queries.remove(&tab_id);
        self.redis_key_folder_visible_cache.remove(&tab_id);
        self.redis_search_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        self.sync_redis_search_controls(tab_id, window, cx);
        self.refresh_active_data_table(tab_id, cx);
        cx.notify();
    }

    /// 解析 Redis Key 列表 Tab 对应的 (连接 ID, DB 索引)，`database` 为 `None` 时按 `"0"` 处理。
    fn redis_tab_connection(&self, tab_id: TabId) -> Option<(ConnectionId, String)> {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor)
                    if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
                {
                    Some((
                        editor.object.connection_id,
                        editor.object.database.clone().unwrap_or_else(|| "0".to_string()),
                    ))
                }
                _ => None,
            })
    }

    /// 惰性加载指定 Tab 的连接 + DB 的搜索历史（仅首次打开时读一次存储）。
    fn ensure_redis_key_search_history_loaded(&mut self, tab_id: TabId) {
        let Some(key) = self.redis_tab_connection(tab_id) else {
            return;
        };
        if self.redis_key_search_history_loaded.contains_key(&key) {
            return;
        }
        let records = self
            .storage
            .load_redis_key_search_history()
            .unwrap_or_default();
        let mut history: BTreeMap<(ConnectionId, String), Vec<String>> = BTreeMap::new();
        for record in records {
            let db = record.database.unwrap_or_else(|| "0".to_string());
            let list = history.entry((record.connection_id, db)).or_default();
            if !list.contains(&record.text) {
                list.push(record.text);
            }
        }
        self.redis_key_search_history = history;
        self.redis_key_search_history_loaded.insert(key, true);
    }

    /// 读取某 Tab 所属连接 + DB 的搜索历史词（队首最新，空表示无历史）。
    fn redis_key_search_history_for(&self, tab_id: TabId) -> Vec<String> {
        let Some(key) = self.redis_tab_connection(tab_id) else {
            return Vec::new();
        };
        self.redis_key_search_history
            .get(&key)
            .cloned()
            .unwrap_or_default()
    }

    /// 搜索应用成功后，把搜索词写入当前连接 + DB 的历史（去重移队首，上限 200），并落盘标记。
    fn record_redis_key_search_history(&mut self, tab_id: TabId) {
        let Some(key) = self.redis_tab_connection(tab_id) else {
            return;
        };
        let query = self.redis_search_query(tab_id).to_string();
        if query.is_empty() {
            return;
        }
        let list = self.redis_key_search_history.entry(key.clone()).or_default();
        if let Some(index) = list.iter().position(|item| item == &query) {
            list.remove(index);
        }
        list.insert(0, query);
        list.truncate(SEARCH_HISTORY_LIMIT);
        self.redis_key_search_history_loaded.insert(key, true);
        self.flush_redis_key_search_history();
    }

    /// 把内存中的全部 Redis Key 搜索历史落盘。
    fn flush_redis_key_search_history(&self) {
        let records = self
            .redis_key_search_history
            .iter()
            .flat_map(|((connection_id, database), texts)| {
                texts.iter().map(move |text| RedisKeySearchHistoryRecord {
                    connection_id: *connection_id,
                    database: Some(database.clone()),
                    text: text.clone(),
                })
            })
            .collect::<Vec<_>>();
        let _ = self.storage.save_redis_key_search_history(&records);
    }

    /// 清除当前 Tab 所属连接 + DB 的搜索历史并落盘。
    fn clear_redis_key_search_history(&mut self, tab_id: TabId) {
        let Some(key) = self.redis_tab_connection(tab_id) else {
            return;
        };
        self.redis_key_search_history.remove(&key);
        self.redis_key_search_history_loaded.insert(key, true);
        self.flush_redis_key_search_history();
    }

    /// 切换搜索历史下拉开关，并确保该 Tab 历史已加载。
    fn toggle_redis_key_search_history(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.ensure_redis_key_search_history_loaded(tab_id);
        self.redis_key_search_history_open = !self.redis_key_search_history_open;
        cx.notify();
    }

    /// 点击历史词：回填搜索框并立即应用搜索。
    fn apply_redis_key_search_history(
        &mut self,
        tab_id: TabId,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.redis_key_search_history_open = false;
        self.redis_search_drafts.insert(tab_id, text.clone());
        self.redis_search_input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
        });
        self.apply_redis_search(tab_id, cx);
    }

    /// 关闭搜索历史下拉。
    fn close_redis_key_search_history(&mut self, cx: &mut Context<Self>) {
        self.redis_key_search_history_open = false;
        cx.notify();
    }

    fn set_redis_type_filter(&mut self, tab_id: TabId, value: String, cx: &mut Context<Self>) {
        self.redis_key_folder_visible_cache.remove(&tab_id);
        if value == REDIS_TYPE_FILTER_ALL {
            self.redis_type_filters.remove(&tab_id);
        } else {
            self.redis_type_filters.insert(tab_id, value);
        }
        self.refresh_active_data_table(tab_id, cx);
        cx.notify();
    }

    fn sync_redis_search_controls(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let expected = self.redis_search_draft(tab_id).to_string();
        let input_focused = self
            .redis_search_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if !input_focused && self.redis_search_input.read(cx).value().to_string() != expected {
            self.redis_search_input.update(cx, |input, cx| {
                input.set_value(expected, window, cx);
            });
        }
        self.redis_type_select.update(cx, |select, cx| {
            select.set_selected_index(
                redis_option_index(&redis_type_filter_options(), self.redis_type_filter(tab_id)),
                window,
                cx,
            );
        });
    }

    fn toggle_field_filter_popover(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.field_filter_popover = if self.field_filter_popover == Some(tab_id) {
            None
        } else {
            self.data_filter_popover = None;
            self.local_filter_popover = None;
            self.local_filter_manager_popover = None;
            Some(tab_id)
        };
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    fn visible_fields_for_page(&self, tab_id: TabId, page: &DataPage) -> BTreeSet<String> {
        self.visible_table_fields
            .get(&tab_id)
            .cloned()
            .unwrap_or_else(|| {
                page.columns
                    .iter()
                    .map(|column| column.name.clone())
                    .collect::<BTreeSet<_>>()
            })
    }

    fn refresh_visible_fields_table(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(tab) = self.controller.state().active_tab().cloned() else {
            return;
        };
        if tab.id != tab_id {
            return;
        }
        let TabKind::DataEditor(editor) = tab.kind else {
            return;
        };
        if let Some(page) = editor.page.as_ref() {
            self.refresh_data_table_state(tab.id, page, cx);
        }
        cx.notify();
    }

    fn toggle_visible_table_field(
        &mut self,
        tab_id: TabId,
        all_fields: Vec<String>,
        field: String,
        cx: &mut Context<Self>,
    ) {
        let visible = self
            .visible_table_fields
            .entry(tab_id)
            .or_insert_with(|| all_fields.iter().cloned().collect::<BTreeSet<_>>());
        if visible.contains(&field) {
            if visible.len() > 1 {
                visible.remove(&field);
            }
        } else {
            visible.insert(field);
        }
        self.refresh_visible_fields_table(tab_id, cx);
    }

    fn invert_visible_table_fields(
        &mut self,
        tab_id: TabId,
        all_fields: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .visible_table_fields
            .get(&tab_id)
            .cloned()
            .unwrap_or_else(|| all_fields.iter().cloned().collect::<BTreeSet<_>>());
        let mut inverted = all_fields
            .iter()
            .filter(|field| !current.contains(*field))
            .cloned()
            .collect::<BTreeSet<_>>();
        if inverted.is_empty() {
            if let Some(first) = all_fields.first() {
                inverted.insert(first.clone());
            }
        }
        self.visible_table_fields.insert(tab_id, inverted);
        self.refresh_visible_fields_table(tab_id, cx);
    }

    fn show_all_table_fields(
        &mut self,
        tab_id: TabId,
        all_fields: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.visible_table_fields
            .insert(tab_id, all_fields.into_iter().collect());
        self.refresh_visible_fields_table(tab_id, cx);
    }

    fn update_data_filter_manual_value(&mut self, value: String, cx: &mut Context<Self>) {
        self.data_filter_value_input_text = value.clone();
        let Some(popover) = self.data_filter_popover else {
            cx.notify();
            return;
        };
        if popover.kind != DataFilterPopoverKind::Value {
            cx.notify();
            return;
        }

        let value = value.trim().to_string();
        let rule_index = popover.rule_index.unwrap_or(0);
        if let Some(rule) = self
            .data_filter_draft_rules
            .get_mut(&popover.tab_id)
            .and_then(|rules| rules.get_mut(rule_index))
        {
            rule.values.clear();
            if !value.is_empty() {
                rule.values.insert(value);
            }
        }
        cx.notify();
    }

    fn active_data_filter_tab_id(&self) -> Option<TabId> {
        self.controller
            .state()
            .active_tab()
            .map(|tab| tab.id)
            .filter(|tab_id| self.data_filter_panels.contains(tab_id))
    }

    fn active_data_editor_tab_id(&self) -> Option<TabId> {
        self.controller
            .state()
            .active_tab()
            .filter(|tab| matches!(tab.kind, TabKind::DataEditor(_)))
            .map(|tab| tab.id)
    }

    fn active_searchable_data_tab_id(&self) -> Option<TabId> {
        self.controller
            .state()
            .active_tab()
            .filter(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => editor.page.is_some(),
                TabKind::QueryEditor(editor) => self
                    .query_output_tabs
                    .get(&tab.id)
                    .copied()
                    .unwrap_or(QueryOutputTab::Result(0))
                    .result_index()
                    .is_some_and(|index| query_result_page_index(editor, index).is_some()),
                _ => false,
            })
            .map(|tab| tab.id)
    }

    fn open_data_search(
        &mut self,
        _: &OpenDataSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.active_searchable_data_tab_id() else {
            return;
        };
        self.data_search_panels.insert(tab_id);
        self.data_search_highlight_all_tabs.insert(tab_id);
        let value = self
            .data_search_queries
            .get(&tab_id)
            .cloned()
            .unwrap_or_default();
        self.data_search_input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        self.refresh_active_data_table(tab_id, cx);
        cx.notify();
    }

    fn toggle_data_search_panel(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.data_search_panels.remove(&tab_id) {
            self.clear_data_search_state(tab_id, window, cx);
        } else {
            self.data_search_panels.insert(tab_id);
            self.data_search_highlight_all_tabs.insert(tab_id);
            let value = self
                .data_search_queries
                .get(&tab_id)
                .cloned()
                .unwrap_or_default();
            self.data_search_input.update(cx, |input, cx| {
                input.set_value(value, window, cx);
                input.focus(window, cx);
            });
        }
        self.refresh_active_data_table(tab_id, cx);
        cx.notify();
    }

    fn clear_data_search_state(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.data_search_queries.remove(&tab_id);
        self.data_search_active_matches.remove(&tab_id);
        self.data_search_highlight_all_tabs.remove(&tab_id);
        self.data_search_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
    }

    fn close_data_search_panel(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.data_search_panels.remove(&tab_id);
        self.clear_data_search_state(tab_id, window, cx);
        self.refresh_active_data_table(tab_id, cx);
        cx.notify();
    }

    fn update_data_search_query(&mut self, value: String, cx: &mut Context<Self>) {
        let Some(tab_id) = self.active_searchable_data_tab_id() else {
            return;
        };
        if value.trim().is_empty() {
            self.data_search_queries.remove(&tab_id);
        } else {
            self.data_search_queries.insert(tab_id, value);
        }
        self.data_search_active_matches.remove(&tab_id);
        self.refresh_active_data_table(tab_id, cx);
        cx.notify();
    }

    fn toggle_data_search_highlight_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if !self.data_search_highlight_all_tabs.remove(&tab_id) {
            self.data_search_highlight_all_tabs.insert(tab_id);
        }
        self.refresh_active_data_table(tab_id, cx);
        cx.notify();
    }

    fn select_next_data_search_match(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(table_state) = self.data_table_states.get(&tab_id).cloned() else {
            return;
        };
        table_state.update(cx, |table, cx| {
            let matches = table.delegate().search_matches.clone();
            let next = next_data_search_match(
                matches.as_slice(),
                table
                    .delegate()
                    .active_search_match
                    .or_else(|| self.data_search_active_matches.get(&tab_id).copied()),
            );
            let Some(next) = next else {
                return;
            };
            self.data_search_active_matches.insert(tab_id, next);
            let delegate = table.delegate_mut();
            delegate.active_search_match = Some(next);
            delegate.select_cell_for_click(next.row_ix, next.col_ix, false);
            table.scroll_to_row(next.row_ix, cx);
            table.scroll_to_col(next.col_ix, cx);
            table.refresh(cx);
        });
        cx.notify();
    }

}

const REDIS_TYPE_FILTER_ALL: &str = "所有";
/// 单连接 + 单 DB 的 Redis Key 搜索历史上限（保留最近 N 条）。
const SEARCH_HISTORY_LIMIT: usize = 200;

fn redis_type_filter_options() -> Vec<String> {
    ["所有", "string", "list", "set", "zset", "hash", "stream", "json"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// 在模式搜索列表中定位选项索引；未命中时回退到首项。
fn redis_option_index(options: &[String], value: &str) -> Option<IndexPath> {
    options
        .iter()
        .position(|option| option == value)
        .map(IndexPath::new)
        .or_else(|| Some(IndexPath::new(0)))
}

/// 按类型过滤 + 键名匹配过滤，检索逻辑对齐 RedisInsight 的键搜索（无包含/前缀/后缀等显式模式，
/// 统一按 Redis `SCAN MATCH` 通配符语义匹配，空查询表示不过滤）。
fn redis_filter_page(page: &DataPage, type_filter: &str, query: &str) -> DataPage {
    let needle = query.to_ascii_lowercase();
    DataPage {
        columns: page.columns.clone(),
        rows: page
            .rows
            .iter()
            .filter(|row| redis_row_matches(row, type_filter, &needle))
            .cloned()
            .collect(),
        offset: page.offset,
        limit: page.limit,
        has_more: page.has_more,
    }
}

fn redis_row_matches(row: &Row, type_filter: &str, query: &str) -> bool {
    let key = row
        .values
        .first()
        .map(CellValue::display_label)
        .unwrap_or_default();
    let kind = row
        .values
        .get(1)
        .map(CellValue::display_label)
        .unwrap_or_default()
        .to_ascii_lowercase();
    (type_filter == REDIS_TYPE_FILTER_ALL || kind == type_filter)
        && (query.is_empty() || redis_glob_matches(&key, query))
}

/// Redis `SCAN MATCH` 通配符匹配：`*` 匹配零或多个任意字符，`?` 匹配单个任意字符，
/// 其余字符按字面匹配。键名不转小写（Redis 键区分大小写），查询已在调用侧转小写。
fn redis_glob_matches(key: &str, pattern_lower: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let k = key.as_bytes();
    let p = pattern_lower.as_bytes();
    // dp[i][j]：pattern 前 i 个字符是否匹配 key 前 j 个字符。
    let mut dp = vec![vec![false; k.len() + 1]; p.len() + 1];
    dp[0][0] = true;
    for i in 1..=p.len() {
        if p[i - 1] == b'*' {
            dp[i][0] = dp[i - 1][0];
        }
    }
    for i in 1..=p.len() {
        for j in 1..=k.len() {
            dp[i][j] = match p[i - 1] {
                b'*' => dp[i - 1][j] || dp[i][j - 1],
                b'?' => dp[i - 1][j - 1],
                c => dp[i - 1][j - 1] && c == k[j - 1],
            };
        }
    }
    dp[p.len()][k.len()]
}
