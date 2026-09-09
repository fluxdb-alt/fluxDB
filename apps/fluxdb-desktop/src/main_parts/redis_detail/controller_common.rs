impl NavicatMain {
    fn ensure_redis_refresh_time_ticker(&mut self, cx: &mut Context<Self>) {
        if self.redis_refresh_time_task.is_some() {
            return;
        }

        self.redis_refresh_time_task = Some(cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(30))
                    .await;
                let should_continue = match view.update(cx, |this, cx| {
                    let has_refresh_times = !this.redis_data_refresh_times.is_empty();
                    if has_refresh_times {
                        cx.notify();
                    } else {
                        this.redis_refresh_time_task = None;
                    }
                    has_refresh_times
                }) {
                    Ok(should_continue) => should_continue,
                    Err(_) => break,
                };
                if !should_continue {
                    break;
                }
            }
        }));
    }

    fn redis_key_value_input_for_kind(&self, kind: &str) -> Entity<InputState> {
        if redis_key_value_is_json_kind(kind) {
            self.redis_json_key_value_input.clone()
        } else {
            self.redis_key_value_input.clone()
        }
    }

    fn redis_key_name_display(&self, tab_id: TabId, detail: &RedisKeyDetail) -> String {
        self.redis_key_name_drafts
            .get(&(tab_id, detail.key.clone()))
            .cloned()
            .unwrap_or_else(|| detail.key.clone())
    }

    fn redis_key_ttl_display(&self, tab_id: TabId, detail: &RedisKeyDetail) -> String {
        self.redis_key_ttl_drafts
            .get(&(tab_id, detail.key.clone()))
            .cloned()
            .unwrap_or_else(|| detail.ttl.clone())
    }

    fn redis_key_ttl_draft_value(&self, tab_id: TabId, detail: &RedisKeyDetail) -> String {
        self.redis_key_ttl_drafts
            .get(&(tab_id, detail.key.clone()))
            .cloned()
            .unwrap_or_else(|| redis_ttl_input_value(&detail.ttl))
    }

    fn redis_key_meta_dirty(&self, tab_id: TabId, detail: &RedisKeyDetail) -> bool {
        self.redis_key_name_display(tab_id, detail) != detail.key
            || self.redis_key_ttl_draft_value(tab_id, detail) != redis_ttl_input_value(&detail.ttl)
    }

    fn redis_key_value_dirty(
        &self,
        tab_id: TabId,
        detail: &RedisKeyDetail,
        input: &Entity<InputState>,
        cx: &App,
    ) -> bool {
        if !redis_key_value_editable_for(&detail.kind, &detail.value) {
            return false;
        }
        let key = detail.key.clone();
        // 只有显式进入 string 编辑态才判定 value dirty：比较输入框与「已完整加载值」，
        // 而不是 200 字符 preview，避免未编辑时误判为有改动。
        if self.redis_string_editing != Some((tab_id, key.clone())) {
            return false;
        }
        let loaded = self
            .redis_string_values
            .get(&(tab_id, key))
            .map(|state| state.value.clone())
            .unwrap_or_else(|| detail.value.clone());
        input.read(cx).value().as_ref() != loaded.as_str()
    }

    fn sync_redis_key_meta_inputs(
        &mut self,
        tab_id: TabId,
        detail: &RedisKeyDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = (tab_id, detail.key.clone());
        let key_expected = self.redis_key_name_display(tab_id, detail);
        let ttl_expected = self
            .redis_key_ttl_drafts
            .get(&active)
            .cloned()
            .unwrap_or_else(|| redis_ttl_input_value(&detail.ttl));
        let active_changed = self.redis_key_meta_active.as_ref() != Some(&active);
        let key_focused = self
            .redis_key_name_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let ttl_focused = self
            .redis_key_ttl_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);

        self.redis_key_meta_syncing = true;
        if active_changed
            || (!key_focused
                && self.redis_key_name_input.read(cx).value().as_ref() != key_expected.as_str())
        {
            self.redis_key_name_input.update(cx, |input, cx| {
                input.set_value(key_expected, window, cx);
            });
        }
        if active_changed
            || (!ttl_focused
                && self.redis_key_ttl_input.read(cx).value().as_ref() != ttl_expected.as_str())
        {
            self.redis_key_ttl_input.update(cx, |input, cx| {
                input.set_value(ttl_expected, window, cx);
            });
        }
        self.redis_key_meta_syncing = false;
        self.redis_key_meta_active = Some(active);
    }

    fn active_redis_selected_key(&self, cx: &App) -> Option<(TabId, String)> {
        self.active_redis_selected_detail(cx)
            .map(|(tab_id, detail)| (tab_id, detail.key))
    }

    fn active_redis_selected_detail(&self, cx: &App) -> Option<(TabId, RedisKeyDetail)> {
        let tab = self.controller.state().active_tab()?;
        let TabKind::DataEditor(editor) = &tab.kind else {
            return None;
        };
        if !matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) {
            return None;
        }
        let page = editor.page.as_ref()?;
        let table_state = self.data_table_states.get(&tab.id)?;
        let selected_row = {
            let table = table_state.read(cx);
            let delegate = table.delegate();
            delegate
                .selected_row
                .or_else(|| delegate.selected_cell.map(|(row, _)| row))
                .and_then(|row| delegate.source_row_indexes.get(row).copied())
        }?;
        redis_key_detail_for_row(page, selected_row).map(|detail| (tab.id, detail))
    }

    fn sync_redis_key_value_input(
        &mut self,
        tab_id: TabId,
        detail: &RedisKeyDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = (tab_id, detail.key.clone());
        // 输入框基线：编辑草稿 > 已加载值 > 列表 preview，避免把 200 字符 preview 写回输入框。
        let expected = self
            .redis_key_value_drafts
            .get(&active)
            .cloned()
            .or_else(|| {
                self.redis_string_values
                    .get(&active)
                    .map(|state| state.value.clone())
            })
            .unwrap_or_else(|| detail.value.clone());
        let active_changed = self.redis_key_value_active.as_ref() != Some(&active);
        let value_input = self.redis_key_value_input_for_kind(&detail.kind);
        let focused = value_input.read(cx).focus_handle(cx).is_focused(window);
        if active_changed
            || (!focused && value_input.read(cx).value().as_ref() != expected.as_str())
        {
            self.redis_key_value_syncing = true;
            value_input.update(cx, |input, cx| {
                input.set_value(expected, window, cx);
            });
            self.redis_key_value_syncing = false;
        }
        self.redis_key_value_active = Some(active);
    }

    fn discard_redis_key_drafts(
        &mut self,
        tab_id: TabId,
        detail: RedisKeyDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = detail.key.clone();
        self.redis_key_name_drafts.remove(&(tab_id, key.clone()));
        self.redis_key_ttl_drafts.remove(&(tab_id, key.clone()));
        self.redis_key_value_drafts.remove(&(tab_id, key.clone()));
        self.redis_key_meta_editing = None;
        self.redis_string_editing = None;
        self.redis_key_meta_active = Some((tab_id, key.clone()));
        self.sync_redis_key_meta_inputs(tab_id, &detail, window, cx);
        if detail.kind.eq_ignore_ascii_case("set") {
            self.pending_redis_set_member_delete = None;
            self.discard_redis_set_member_rows(tab_id, &detail, window, cx);
        }
        self.redis_key_value_syncing = true;
        self.redis_key_value_input_for_kind(&detail.kind).update(cx, |input, cx| {
            input.set_value(detail.value, window, cx);
        });
        self.redis_key_value_syncing = false;
        self.redis_key_value_active = Some((tab_id, key));
        cx.notify();
    }

    fn request_redis_key_value_apply(
        &mut self,
        tab_id: TabId,
        detail: RedisKeyDetail,
        cx: &mut Context<Self>,
    ) {
        let key = detail.key.clone();
        let new_key = if self.redis_key_meta_editing == Some(RedisKeyMetaField::KeyName) {
            self.redis_key_name_input.read(cx).value().to_string()
        } else {
            self.redis_key_name_display(tab_id, &detail)
        };
        if new_key.trim().is_empty() {
            self.show_message("键名称不能为空", AppMessageKind::Warning, cx);
            return;
        }
        let ttl_draft = if self.redis_key_meta_editing == Some(RedisKeyMetaField::Ttl) {
            self.redis_key_ttl_input
                .read(cx)
                .value()
                .chars()
                .filter(|ch| ch.is_ascii_digit())
                .collect::<String>()
        } else {
            self.redis_key_ttl_draft_value(tab_id, &detail)
        };
        let ttl = (ttl_draft != redis_ttl_input_value(&detail.ttl)).then_some(ttl_draft);
        // ReJSON / JSON 键：以编辑器 `source` 为权威全文。编辑态下先校验合法性，非法则阻止保存。
        // 与展示分支共用同一套 JSON 类型判断，保证 rejson / rejson-rl 也走 JSON 专用保存准备逻辑。
        let mut json_save_value: Option<Option<String>> = None; // None = 非 JSON 键
        if redis_key_value_is_json_kind(&detail.kind) {
            let source = self.redis_json_editor.source.clone();
            let dirty = self.redis_json_editor.dirty;
            if self.redis_json_editor.editing && self.redis_json_editor_is_active(tab_id, &key) {
                if let Some(dia) = self.redis_json_validate() {
                    self.show_message(dia.summary(), AppMessageKind::Error, cx);
                    return;
                }
            }
            let state = self.redis_string_values.get(&(tab_id, key.clone()));
            let loaded_all = state.map(|state| state.loaded_all).unwrap_or(false);
            if !loaded_all {
                // 未完整加载：禁止以截断内容覆盖完整 JSON。
                self.show_message("请先「加载全部」再保存 JSON", AppMessageKind::Warning, cx);
                return;
            }
            let base = state
                .map(|state| state.value.clone())
                .unwrap_or_else(|| detail.value.clone());
            // 校验 + 是否写回 + format_on_save 规整均由纯函数负责（可单测）。
            match redis_json_save_prepare(
                source,
                base,
                dirty,
                self.redis_json_editor.config.format_on_save,
                self.redis_json_editor.config.indent_size,
            ) {
                Ok(v) => json_save_value = Some(v),
                Err(msg) => {
                    self.show_message(msg, AppMessageKind::Error, cx);
                    return;
                }
            }
        }
        let value = if let Some(json_value) = json_save_value {
            json_value
        } else if detail.kind.eq_ignore_ascii_case("set") {
            let (_, current_members) = redis_set_preview_members(&detail.value);
            let current_members = current_members.into_iter().collect::<BTreeSet<_>>();
            let rows = self.redis_set_member_rows_snapshot(cx);
            if rows.is_empty() {
                self.show_message("至少添加一个 member", AppMessageKind::Warning, cx);
                return;
            }
            if rows.iter().any(|member| member.trim().is_empty()) {
                self.show_message("Member 不能为空", AppMessageKind::Warning, cx);
                return;
            }
            let desired_members = rows.iter().cloned().collect::<BTreeSet<_>>();
            if desired_members == current_members {
                None
            } else {
                Some(desired_members.into_iter().collect::<Vec<_>>().join("
"))
            }
        } else {
            let editable = redis_key_value_editable_for(&detail.kind, &detail.value);
            if !editable {
                None
            } else {
                // 硬性保护：string 值只有 loaded_all 时才以「完整值」为基线比较。
                // 未完整加载时 value 必须为 None，否则会把 preview 片段写回覆盖完整数据。
                let state = self.redis_string_values.get(&(tab_id, key.clone()));
                let loaded_all = state.map(|state| state.loaded_all).unwrap_or(false);
                if !loaded_all || self.redis_string_editing != Some((tab_id, key.clone())) {
                    None
                } else {
                    let base = state
                        .map(|state| state.value.clone())
                        .unwrap_or_else(|| detail.value.clone());
                    let value = self
                        .redis_key_value_input_for_kind(&detail.kind)
                        .read(cx)
                        .value()
                        .to_string();
                    (value != base).then_some(value)
                }
            }
        };
        // 对齐 RedisInsight 的「不要把二进制垃圾 / 异常控制字符写进 key」守卫：键名与
        // 字符串值里若夹带 NUL、ESC 等不可打印控制字符，保存后既难读也易损坏数据。
        // 键名与值都在这里校验（制表符 / 换行 / 回车放行，不影响多行文本）。
        if let Some(ch) = redis_value_illegal_control_char(&new_key) {
            self.show_message(
                format!("键名称包含非法控制字符（U+{:04X}），请删除后再保存", ch as u32),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        if let Some(v) = &value {
            if let Some(ch) = redis_value_illegal_control_char(v) {
                self.show_message(
                    format!("值包含非法控制字符（U+{:04X}），请删除后再保存", ch as u32),
                    AppMessageKind::Warning,
                    cx,
                );
                return;
            }
        }
        if new_key == detail.key && ttl.is_none() && value.is_none() {
            self.show_message("当前 Key 没有修改", AppMessageKind::Warning, cx);
            return;
        }

        let task_id = redis_key_value_apply_task_id(tab_id);
        if self._redis_key_value_apply_tasks.contains_key(&task_id) {
            self.show_message("正在保存当前 Key", AppMessageKind::Warning, cx);
            return;
        }

        let mut controller = self.controller.clone();
        let apply_key = key.clone();
        let apply_new_key = new_key.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::ApplyRedisKeyValue {
                        tab_id,
                        key: apply_key.clone(),
                        new_key: apply_new_key,
                        ttl,
                        value,
                    }) {
                        AppEvent::DataLoaded(_, page) => {
                            page.rows.into_iter().next().ok_or_else(|| {
                                fluxdb_core::UserFacingError {
                                    title: "保存失败".to_string(),
                                    message: "Redis Key 保存后没有返回数据".to_string(),
                                    detail: None,
                                    retryable: true,
                                }
                            })
                        }
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "保存失败".to_string(),
                            message: "Redis Key 保存没有返回结果".to_string(),
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
                            this.redis_key_value_drafts.remove(&(tab_id, key.clone()));
                            this.redis_key_name_drafts.remove(&(tab_id, key.clone()));
                            this.redis_key_ttl_drafts.remove(&(tab_id, key.clone()));
                            this.redis_key_meta_editing = None;
                            this.redis_string_editing = None;
                            // 值已落库，缓存失效：下次打开详情按新值重新加载 preview/完整值。
                            this.redis_string_values.remove(&(tab_id, key.clone()));
                            this.show_message("Key 已保存", AppMessageKind::Success, cx);
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

    fn refresh_redis_key_detail(
        &mut self,
        tab_id: TabId,
        key: String,
        refresh_kind: RedisKeyDetailRefreshKind,
        cx: &mut Context<Self>,
    ) {
        // 刷新按点击时捕获的类型直接分发，避免完成后再读当前选中态把 hash 分支跳掉。
        refresh_kind.refresh(tab_id, key, self, cx);
    }

    fn request_redis_key_refresh(
        &mut self,
        tab_id: TabId,
        key: String,
        refresh_kind: RedisKeyDetailRefreshKind,
        cx: &mut Context<Self>,
    ) {
        // 刷新 key 前自动关闭完整值内嵌面板（即使刷新已在途也先关闭）
        self.close_redis_hash_full_value_viewer(cx);
        let task_id = redis_key_refresh_task_id(tab_id);
        if self._data_load_tasks.contains_key(&task_id) {
            return;
        }

        let mut controller = self.controller.clone();
        let refresh_key = key.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadRedisKey {
                        tab_id,
                        key: refresh_key.clone(),
                    }) {
                        AppEvent::DataLoaded(_, page) => {
                            page.rows.into_iter().next().ok_or_else(|| {
                                fluxdb_core::UserFacingError {
                                    title: "刷新失败".to_string(),
                                    message: "Redis Key 没有返回数据".to_string(),
                                    detail: None,
                                    retryable: true,
                                }
                            })
                        }
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "刷新失败".to_string(),
                            message: "Redis Key 刷新没有返回结果".to_string(),
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
                    let event = this.controller.dispatch(AppCommand::FinishRedisKeyRefresh {
                        tab_id,
                        key: key.clone(),
                        result,
                    });
                    this.apply_app_event(&event, cx);
                    match event {
                        AppEvent::DataLoaded(_, _) => {
                            this.refresh_redis_key_detail(
                                tab_id,
                                key.clone(),
                                refresh_kind,
                                cx,
                            );
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
        self._data_load_tasks.insert(task_id, task);
    }

    /// 解析 Hash 明细标签页对应的连接 id（仅 Redis 数据编辑器标签页有效）。
    fn redis_tab_connection_id(&self, tab_id: TabId) -> Option<ConnectionId> {
        self.controller.state().tabs.iter().find(|tab| tab.id == tab_id).and_then(|tab| {
            match &tab.kind {
                TabKind::DataEditor(editor)
                    if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
                {
                    Some(editor.object.connection_id)
                }
                _ => None,
            }
        })
    }

    /// 惰性探测当前连接的 Redis 服务端版本并写入缓存（字段级 TTL 编辑能力开关）。
    /// 缓存未命中且未在探测中时，dispatch `LoadRedisServerVersion`；用任务 map 去重，
    /// 避免重复渲染重复发起。探测失败同样落 `None`（能力未知，静默降级），
    /// 重连后由 [`NavicatMain`] 清除缓存、下次打开 Hash 面板重新探测。
    fn ensure_redis_server_version_cache(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(connection_id) = self.redis_tab_connection_id(tab_id) else {
            return;
        };
        if self.redis_server_versions.contains_key(&connection_id)
            || self.redis_server_version_tasks.contains_key(&connection_id)
        {
            return;
        }
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let event = cx
                .background_spawn(async move {
                    controller.dispatch(AppCommand::LoadRedisServerVersion(connection_id))
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this.redis_server_version_tasks.remove(&connection_id);
                    let version = match event {
                        AppEvent::RedisServerVersionLoaded(_, version) => version,
                        _ => None,
                    };
                    this.redis_server_versions.insert(connection_id, version);
                    cx.notify();
                });
            });
        });
        self.redis_server_version_tasks.insert(connection_id, task);
    }
}

/// 计算 ReJSON 键保存时应写回的值（纯函数，便于不依赖 Redis 的单测）。
///
/// - `source` 为编辑器权威全文；`base` 为服务端基线；`dirty` 表示是否有未保存修改。
/// - 返回 `Ok(None)`：无修改或与基线一致，无需写回。
/// - 返回 `Ok(Some(v))`：待写回的最终值文本（按 `format_on_save` 规整）。
/// - 返回 `Err(msg)`：JSON 非法，阻止保存并返回面向用户的提示文案。
///
/// 无论查看态还是编辑态，保存前都以 `serde_json` 为权威校验，防止脏数据落库。
fn redis_json_save_prepare(
    source: String,
    base: String,
    dirty: bool,
    format_on_save: bool,
    indent_size: usize,
) -> Result<Option<String>, String> {
    if serde_json::from_str::<serde_json::Value>(&source).is_err() {
        return Err("JSON 格式错误，请修正后再保存".to_string());
    }
    if !dirty || source == base {
        return Ok(None);
    }
    let final_value = if format_on_save {
        format_pretty(&source, indent_size).unwrap_or(source)
    } else {
        source
    };
    Ok(Some(final_value))
}

#[cfg(test)]
mod redis_json_save_prepare_tests {
    use super::*;

    #[test]
    fn json_save_prepare_rejects_invalid_and_blocks() {
        // 非法 JSON 必须阻止保存（编辑态/查看态均生效）。
        let err = redis_json_save_prepare(
            r#"{"a": }"#.to_string(),
            r#"{"a":1}"#.to_string(),
            true,
            true,
            2,
        );
        assert!(err.is_err());
    }

    #[test]
    fn json_save_prepare_noop_when_not_dirty_or_unchanged() {
        // 未修改 / 与基线一致时不写回。
        let none = redis_json_save_prepare(
            r#"{"a":1}"#.to_string(),
            r#"{"a":1}"#.to_string(),
            true,
            true,
            2,
        );
        assert_eq!(none.unwrap(), None);
        let none2 = redis_json_save_prepare(
            r#"{"a":2}"#.to_string(),
            r#"{"a":1}"#.to_string(),
            false,
            true,
            2,
        );
        assert_eq!(none2.unwrap(), None);
    }

    #[test]
    fn json_save_prepare_format_on_save_applies_pretty() {
        // 仅当 dirty 且内容变化时才写回；format_on_save 走标准 pretty。
        let out = redis_json_save_prepare(
            r#"{"a":1,"b":2}"#.to_string(),
            r#"{"a":1}"#.to_string(),
            true,
            true,
            2,
        )
        .unwrap()
        .unwrap();
        assert_eq!(out, "{\n  \"a\": 1,\n  \"b\": 2\n}");
    }

    #[test]
    fn json_save_prepare_keeps_raw_when_format_off() {
        let out = redis_json_save_prepare(
            r#"{"a":1,"b":2}"#.to_string(),
            r#"{"a":1}"#.to_string(),
            true,
            false,
            2,
        )
        .unwrap()
        .unwrap();
        // 未启用 format_on_save 时保持原文。
        assert_eq!(out, r#"{"a":1,"b":2}"#);
    }
}
