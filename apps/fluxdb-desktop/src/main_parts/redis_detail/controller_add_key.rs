// 「新增 Key」控制器（对齐 RedisInsight AddKey）：只做状态编排、校验与提交调度。
// 打开/关闭抽屉、类型切换重置依赖字段、增删类型子表单行、收集并组装 RedisAddKeyRequest、
// 调度 CreateRedisKey；成功后关抽屉并刷新键列表、打开新键详情，失败仅提示错误且不丢表单状态。

impl NavicatMain {
    /// 读取当前选中的建 Key 类型（下拉 value → `RedisAddKeyKind`，未知默认 String）。
    fn redis_add_key_kind(&self, cx: &App) -> RedisAddKeyKind {
        match self
            .redis_add_key_type_select
            .read(cx)
            .selected_value()
            .map(|value| value.as_str())
        {
            Some("Hash") => RedisAddKeyKind::Hash,
            Some("List") => RedisAddKeyKind::List,
            Some("Set") => RedisAddKeyKind::Set,
            Some("ZSet") => RedisAddKeyKind::ZSet,
            Some("Stream") => RedisAddKeyKind::Stream,
            Some("JSON") => RedisAddKeyKind::Json,
            _ => RedisAddKeyKind::String,
        }
    }

    /// 打开「新增 Key」抽屉：类型重置为 String，清空公共字段，重建全部类型子表单的行与输入，方向回到尾插。
    fn open_redis_add_key_drawer(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.redis_add_key_drawer = Some(RedisAddKeyDrawerForm { tab_id });
        self.redis_add_key_applying = false;
        self.redis_add_key_type_select
            .update(cx, |select, cx| {
                select.set_selected_index(Some(IndexPath::new(0)), window, cx);
            });
        for input in [&self.redis_add_key_name_input, &self.redis_add_key_ttl_input] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.redis_add_key_string_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.redis_add_key_json_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.redis_add_key_stream_id_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.redis_add_key_list_direction = RedisListDirection::Tail;
        self.reset_redis_add_key_form(window, cx);
        cx.notify();
    }

    /// 关闭「新增 Key」抽屉（遮罩点击 / 取消按钮 / Escape）。下次打开时自动重建全部子表单。
    fn cancel_redis_add_key_drawer(&mut self, cx: &mut Context<Self>) {
        self.redis_add_key_drawer = None;
        self.redis_add_key_applying = false;
        cx.notify();
    }

    /// 抽屉打开时重置全部类型子表单：每个可增删行集合都重建为一个空行，供各类型使用。
    fn reset_redis_add_key_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.redis_add_key_hash_rows = vec![new_add_key_hash_row(window, cx)];
        self.redis_add_key_zset_rows = vec![new_add_key_pair_row("Member", "Score", window, cx)];
        self.redis_add_key_set_rows = vec![new_add_key_single_row("Member", window, cx)];
        self.redis_add_key_list_rows = vec![new_add_key_single_row("Element", window, cx)];
        self.redis_add_key_stream_rows = vec![new_add_key_pair_row("Field", "Value", window, cx)];
    }

    /// 类型切换时重置「当前所选类型」子表单的依赖字段：清空对应单值输入或重建对应行集合。
    fn reset_redis_add_key_active_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.redis_add_key_kind(cx) {
            RedisAddKeyKind::String => {
                self.redis_add_key_string_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            RedisAddKeyKind::Json => {
                self.redis_add_key_json_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            RedisAddKeyKind::Hash => {
                self.redis_add_key_hash_rows = vec![new_add_key_hash_row(window, cx)];
            }
            RedisAddKeyKind::ZSet => {
                self.redis_add_key_zset_rows = vec![new_add_key_pair_row("Member", "Score", window, cx)];
            }
            RedisAddKeyKind::Set => {
                self.redis_add_key_set_rows = vec![new_add_key_single_row("Member", window, cx)];
            }
            RedisAddKeyKind::List => {
                self.redis_add_key_list_rows = vec![new_add_key_single_row("Element", window, cx)];
                self.redis_add_key_list_direction = RedisListDirection::Tail;
            }
            RedisAddKeyKind::Stream => {
                self.redis_add_key_stream_id_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.redis_add_key_stream_rows = vec![new_add_key_pair_row("Field", "Value", window, cx)];
            }
        }
    }

    /// 为给定类型追加一行（可增删行的子表单共用），并聚焦新行首个输入。
    fn add_redis_add_key_row(&mut self, kind: RedisAddKeyKind, window: &mut Window, cx: &mut Context<Self>) {
        // 先在局部建好新行（行构造不依赖 self），再 push 到对应集合，避免同一 expr 内双重可变借用。
        match kind {
            RedisAddKeyKind::Hash => {
                let row = new_add_key_hash_row(window, cx);
                self.redis_add_key_hash_rows.push(row);
            }
            RedisAddKeyKind::ZSet => {
                let row = new_add_key_pair_row("Member", "Score", window, cx);
                self.redis_add_key_zset_rows.push(row);
            }
            RedisAddKeyKind::Stream => {
                let row = new_add_key_pair_row("Field", "Value", window, cx);
                self.redis_add_key_stream_rows.push(row);
            }
            RedisAddKeyKind::Set => {
                let row = new_add_key_single_row("Member", window, cx);
                self.redis_add_key_set_rows.push(row);
            }
            RedisAddKeyKind::List => {
                let row = new_add_key_single_row("Element", window, cx);
                self.redis_add_key_list_rows.push(row);
            }
            RedisAddKeyKind::String | RedisAddKeyKind::Json => {}
        }
        cx.notify();
    }

    /// 移除指定类型某一索引的行；若删除后仍有剩余行，则聚焦同位置的后续行输入。
    fn remove_redis_add_key_row(
        &mut self,
        kind: RedisAddKeyKind,
        row_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        macro_rules! finish_remove {
            ($rows:expr, $focus:ident) => {{
                if row_index < $rows.len() {
                    $rows.remove(row_index);
                    if !$rows.is_empty() {
                        let focus = row_index.min($rows.len() - 1);
                        $rows[focus].$focus.update(cx, |input, cx| input.focus(window, cx));
                    }
                }
            }};
        }
        match kind {
            RedisAddKeyKind::Hash => finish_remove!(self.redis_add_key_hash_rows, name),
            RedisAddKeyKind::ZSet => finish_remove!(self.redis_add_key_zset_rows, name),
            RedisAddKeyKind::Stream => finish_remove!(self.redis_add_key_stream_rows, name),
            RedisAddKeyKind::Set => finish_remove!(self.redis_add_key_set_rows, value),
            RedisAddKeyKind::List => finish_remove!(self.redis_add_key_list_rows, value),
            RedisAddKeyKind::String | RedisAddKeyKind::Json => {}
        }
        cx.notify();
    }

    /// 设置 List 插入方向（头/尾）。
    fn set_redis_add_key_list_direction(
        &mut self,
        direction: RedisListDirection,
        cx: &mut Context<Self>,
    ) {
        self.redis_add_key_list_direction = direction;
        self.redis_add_key_applying = false;
        cx.notify();
    }

    /// JSON 格式化/粘贴助手：读取 JSON 输入，合法则美化回填，否则提示错误。
    fn format_redis_add_key_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.redis_add_key_json_input.read(cx).value().to_string();
        match redis_pretty_json(&value) {
            Ok(formatted) => {
                self.redis_add_key_json_input.update(cx, |input, cx| {
                    input.set_value(formatted, window, cx);
                });
                self.show_message("JSON 已格式化", AppMessageKind::Success, cx);
            }
            Err(_) => {
                self.show_message("JSON 内容不合法，无法格式化", AppMessageKind::Warning, cx);
            }
        }
        cx.notify();
    }

    /// 读取名称=值型行集合快照（Hash/ZSet/Stream），返回去空格后的 `(name, value)` 对。
    fn redis_add_key_pair_rows_snapshot(
        &self,
        rows: &[RedisAddKeyNameValueRow],
        cx: &App,
    ) -> Vec<(String, String)> {
        rows.iter()
            .filter_map(|row| {
                let name = row.name.read(cx).value().trim().to_string();
                let value = row.value.read(cx).value().to_string();
                if name.is_empty() && value.trim().is_empty() {
                    None
                } else {
                    Some((name, value))
                }
            })
            .collect()
    }

    /// 读取单成员型行集合快照（Set/List），返回非空成员列表。
    fn redis_add_key_single_rows_snapshot(&self, rows: &[RedisAddKeySingleRow], cx: &App) -> Vec<String> {
        rows.iter()
            .filter_map(|row| {
                let value = row.value.read(cx).value().trim().to_string();
                (!value.is_empty()).then_some(value)
            })
            .collect()
    }

    /// 读取 Hash 行快照：返回去空格后的 `(field, value, ttl)`；ttl 为原始字符串，
    /// 空串表示不过期，非空在 `apply_redis_add_key` 里再校验并解析秒数。全空行跳过。
    fn redis_add_key_hash_rows_snapshot(
        &self,
        rows: &[RedisAddKeyHashRow],
        cx: &App,
    ) -> Vec<(String, String, String)> {
        rows.iter()
            .filter_map(|row| {
                let field = row.name.read(cx).value().trim().to_string();
                let value = row.value.read(cx).value().to_string();
                let ttl = row.ttl.read(cx).value().trim().to_string();
                if field.is_empty() && value.trim().is_empty() && ttl.is_empty() {
                    None
                } else {
                    Some((field, value, ttl))
                }
            })
            .collect()
    }

    /// 点击底部「新建」：读取与校验当前类型子表单 → 组装请求 → 调度 CreateRedisKey。
    /// 校验失败或后端失败只提示错误、不关闭抽屉，表单状态保留。
    fn apply_redis_add_key(&mut self, tab_id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if self.redis_add_key_applying
            || !self
                .redis_add_key_drawer
                .as_ref()
                .is_some_and(|drawer| drawer.tab_id == tab_id)
        {
            return;
        }
        let kind = self.redis_add_key_kind(cx);
        let name = self.redis_add_key_name_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.show_message("Redis Key 名称不能为空", AppMessageKind::Warning, cx);
            return;
        }
        let ttl = self.redis_add_key_ttl_input.read(cx).value().to_string();

        // 按当前类型组装 value / pairs / 字段级 TTL / direction / entry id，并完成该类型的客户端校验。
        let (value, pairs, hash_field_ttls, list_direction, stream_entry_id) = match kind {
            RedisAddKeyKind::String => {
                let value = self.redis_add_key_string_input.read(cx).value().to_string();
                (value, Vec::new(), None, None, None)
            }
            RedisAddKeyKind::Json => {
                let value = self.redis_add_key_json_input.read(cx).value().trim().to_string();
                if value.is_empty() || serde_json::from_str::<serde_json::Value>(&value).is_err() {
                    self.show_message("JSON 内容不合法，请输入合法的 JSON", AppMessageKind::Warning, cx);
                    return;
                }
                (value, Vec::new(), None, None, None)
            }
            RedisAddKeyKind::Hash => {
                let rows = self.redis_add_key_hash_rows_snapshot(&self.redis_add_key_hash_rows, cx);
                if rows.is_empty() {
                    self.show_message("新建 Hash 至少需要一个字段", AppMessageKind::Warning, cx);
                    return;
                }
                if rows.iter().any(|(field, _, _)| field.is_empty()) {
                    self.show_message("字段名不能为空", AppMessageKind::Warning, cx);
                    return;
                }
                // 字段级 TTL：留空不过期，非空必须为 >0 秒（文案对齐 Hash 详情「新增字段」抽屉）。
                if rows.iter().any(|(_, _, ttl)| {
                    !ttl.is_empty() && ttl.parse::<u64>().ok().filter(|secs| *secs > 0).is_none()
                }) {
                    self.show_message(
                        "Redis TTL 必须是秒数，且大于 0 秒或留空",
                        AppMessageKind::Warning,
                        cx,
                    );
                    return;
                }
                let pairs = rows
                    .iter()
                    .map(|(field, value, _)| (field.clone(), value.clone()))
                    .collect::<Vec<_>>();
                // 与 pairs 平行：空串 → None（不过期），非空（已校验 >0）→ Some(secs)。
                let ttls = rows
                    .iter()
                    .map(|(_, _, ttl)| {
                        if ttl.is_empty() {
                            None
                        } else {
                            ttl.parse::<u64>().ok()
                        }
                    })
                    .collect::<Vec<_>>();
                (String::new(), pairs, Some(ttls), None, None)
            }
            RedisAddKeyKind::ZSet | RedisAddKeyKind::Stream => {
                let rows = match kind {
                    RedisAddKeyKind::ZSet => &self.redis_add_key_zset_rows,
                    _ => &self.redis_add_key_stream_rows,
                };
                let pairs = self.redis_add_key_pair_rows_snapshot(rows, cx);
                if pairs.is_empty() {
                    let what = format!("{:?}", kind);
                    self.show_message(format!("新建 {what} 至少需要一个字段"), AppMessageKind::Warning, cx);
                    return;
                }
                if pairs.iter().any(|(name, _)| name.is_empty()) {
                    self.show_message("字段名/成员名不能为空", AppMessageKind::Warning, cx);
                    return;
                }
                if kind == RedisAddKeyKind::ZSet
                    && pairs.iter().any(|(_, score)| score.trim().parse::<f64>().is_err())
                {
                    self.show_message("ZSet 的 score 必须是数字", AppMessageKind::Warning, cx);
                    return;
                }
                let stream_entry_id = if kind == RedisAddKeyKind::Stream {
                    let id = self.redis_add_key_stream_id_input.read(cx).value().trim().to_string();
                    if !id.is_empty() && id != "*" && !is_valid_stream_entry_id(&id) {
                        self.show_message("Stream entry id 需为 `毫秒-序号` 或留空", AppMessageKind::Warning, cx);
                        return;
                    }
                    Some(id)
                } else {
                    None
                };
                (String::new(), pairs, None, None, stream_entry_id)
            }
            RedisAddKeyKind::List => {
                let members = self.redis_add_key_single_rows_snapshot(&self.redis_add_key_list_rows, cx);
                if members.is_empty() {
                    self.show_message("新建 List 至少需要一个元素", AppMessageKind::Warning, cx);
                    return;
                }
                (
                    members.join("\n"),
                    Vec::new(),
                    None,
                    Some(self.redis_add_key_list_direction),
                    None,
                )
            }
            RedisAddKeyKind::Set => {
                let members = self.redis_add_key_single_rows_snapshot(&self.redis_add_key_set_rows, cx);
                if members.is_empty() {
                    self.show_message("新建 Set 至少需要一个成员", AppMessageKind::Warning, cx);
                    return;
                }
                (members.join("\n"), Vec::new(), None, None, None)
            }
        };

        let request = RedisAddKeyRequest {
            key: name,
            kind,
            value,
            pairs,
            ttl: String::new(),
            hash_field_ttls,
            list_direction,
            stream_entry_id,
        };
        self.redis_add_key_applying = true;
        self.start_redis_add_key_mutation(tab_id, request, ttl, cx);
        let _ = window;
    }

    /// 调度建 Key：成功后关抽屉、刷新键列表并打开新键详情；失败保持抽屉且弹错误提示。
    fn start_redis_add_key_mutation(
        &mut self,
        tab_id: TabId,
        request: RedisAddKeyRequest,
        ttl: String,
        cx: &mut Context<Self>,
    ) {
        let task_id = redis_add_key_mutation_task_id(tab_id);
        if self._data_load_tasks.contains_key(&task_id) {
            return;
        }
        let mut controller = self.controller.clone();
        let new_key = request.key.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn({
                    let mut request = request;
                    request.ttl = ttl;
                    async move {
                        match controller.dispatch(AppCommand::CreateRedisKey { tab_id, request }) {
                            AppEvent::RedisKeyCreated { object, key, .. } => Ok((object, key)),
                            AppEvent::Failed(error) => Err(error),
                            _ => Err(fluxdb_core::UserFacingError {
                                title: "新建失败".to_string(),
                                message: "新建 Key 没有返回结果".to_string(),
                                detail: None,
                                retryable: true,
                            }),
                        }
                    }
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._data_load_tasks.remove(&task_id);
                    this.redis_add_key_applying = false;
                    match result {
                        Ok((object, key)) => {
                            // 创建成功：关闭建 Key 抽屉（下次打开时自动重建子表单）。
                            this.redis_add_key_drawer = None;
                            // 刷新 DB 键列表，让新键出现在列表里。
                            this.request_data_editor_refresh(tab_id, cx);
                            // 对齐 RedisInsight：创建后打开新键详情。
                            let key_path = ObjectPath {
                                kind: ObjectKind::RedisKey,
                                name: key.clone(),
                                ..object
                            };
                            this.dispatch(AppCommand::OpenDataEditor(key_path), cx);
                            this.show_message(
                                format!("已创建 Key「{}」", new_key),
                                AppMessageKind::Info,
                                cx,
                            );
                        }
                        Err(error) => {
                            // 失败仅提示错误、保留抽屉表单状态，便于修改后重试。
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
}

/// 新建 名称=值 型行（Hash/ZSet/Stream），并聚焦首个输入框。
fn new_add_key_pair_row(
    name_placeholder: &'static str,
    value_placeholder: &'static str,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> RedisAddKeyNameValueRow {
    let row = RedisAddKeyNameValueRow {
        name: new_redis_add_key_name_input(window, cx, name_placeholder),
        value: new_redis_add_key_pair_value_input(window, cx, value_placeholder),
    };
    row.name.update(cx, |input, cx| input.focus(window, cx));
    row
}

/// 新建 Hash 行：字段名 / 值 / 可选字段级 TTL（秒），并聚焦字段名输入框。
fn new_add_key_hash_row(window: &mut Window, cx: &mut Context<NavicatMain>) -> RedisAddKeyHashRow {
    let row = RedisAddKeyHashRow {
        name: new_redis_add_key_name_input(window, cx, "Field"),
        value: new_redis_add_key_pair_value_input(window, cx, "Value"),
        ttl: new_redis_hash_ttl_input(window, cx, None),
    };
    row.name.update(cx, |input, cx| input.focus(window, cx));
    row
}

/// 新建 单成员 型行（Set/List），并聚焦成员输入框。
fn new_add_key_single_row(
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> RedisAddKeySingleRow {
    let row = RedisAddKeySingleRow {
        value: new_redis_add_key_single_input(window, cx, placeholder),
    };
    row.value.update(cx, |input, cx| input.focus(window, cx));
    row
}

/// Stream entry id 是否为合法的 `毫秒-序号` 形式（供客户端预校验）。
fn is_valid_stream_entry_id(id: &str) -> bool {
    let mut parts = id.splitn(2, '-');
    let ms = parts.next().unwrap_or_default();
    let seq = parts.next().unwrap_or_default();
    !ms.is_empty() && ms.chars().all(|ch| ch.is_ascii_digit()) && seq.chars().all(|ch| ch.is_ascii_digit())
}

/// AddKey 建 Key 任务在 `_data_load_tasks` 里的键：每位 (tab, key) 一个，用独立高位比特隔离。
fn redis_add_key_mutation_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 52)
}
