/// 数据页 SQL 面板里解析出的 LIMIT 的封顶处理。
///
/// 封顶值取「默认分页行数」设置的最大档（见 `data_table_page_size_max`）—— 面板里显示的
/// LIMIT 正是 app 按当前页大小生成的，刷新（⌘R）时会重新解析回来。二者若不同源，
/// 设置选 500/1000 的表一刷新就会掉回封顶值。
fn data_editor_effective_limit(limit: u64) -> u64 {
    limit.clamp(1, data_table_page_size_max())
}

impl NavicatMain {
    fn add_data_filter_rule(
        &mut self,
        tab_id: TabId,
        default_field: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.insert_data_filter_rule(tab_id, None, default_field, 0, cx);
    }

    fn add_data_filter_rule_after(
        &mut self,
        tab_id: TabId,
        rule_index: usize,
        default_field: Option<String>,
        group_depth: u8,
        cx: &mut Context<Self>,
    ) {
        self.insert_data_filter_rule(
            tab_id,
            Some(rule_index + 1),
            default_field,
            group_depth,
            cx,
        );
    }

    fn insert_data_filter_rule(
        &mut self,
        tab_id: TabId,
        insert_at: Option<usize>,
        default_field: Option<String>,
        group_depth: u8,
        cx: &mut Context<Self>,
    ) {
        let rule = DataFilterRule {
            field: default_field,
            group_depth,
            ..DataFilterRule::default()
        };
        let rules = self.data_filter_draft_rules.entry(tab_id).or_default();
        if let Some(insert_at) = insert_at {
            rules.insert(insert_at.min(rules.len()), rule);
        } else {
            rules.push(rule);
        }
        self.data_filter_popover = None;
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    fn toggle_data_filter_rule_enabled(
        &mut self,
        tab_id: TabId,
        rule_index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(rule) = self
            .data_filter_draft_rules
            .get_mut(&tab_id)
            .and_then(|rules| rules.get_mut(rule_index))
        {
            rule.enabled = !rule.enabled;
        }
        cx.notify();
    }

    fn delete_data_filter_rule(
        &mut self,
        tab_id: TabId,
        rule_index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(rules) = self.data_filter_draft_rules.get_mut(&tab_id) {
            if rule_index < rules.len() {
                rules.remove(rule_index);
            }
            if rules.is_empty() {
                self.data_filter_draft_rules.remove(&tab_id);
                self.data_filter_grouped_tabs.remove(&tab_id);
            }
        }
        if self
            .data_filter_popover
            .is_some_and(|popover| popover.tab_id == tab_id)
        {
            self.data_filter_popover = None;
            self.sync_table_hover_overlay_block(cx);
        }
        cx.notify();
    }

    fn add_data_filter_group(
        &mut self,
        tab_id: TabId,
        default_field: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.insert_data_filter_rule(tab_id, None, default_field, 1, cx);
    }

    fn toggle_data_filter_popover(
        &mut self,
        tab_id: TabId,
        rule_index: Option<usize>,
        kind: DataFilterPopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = DataFilterPopover {
            tab_id,
            kind,
            rule_index,
            sort_index: None,
            anchor,
        };
        // 只看「哪个控件的弹层」是否相同，忽略点击像素：同一控件再点一次要能收起。
        let is_same_popover = self.data_filter_popover.is_some_and(|current| {
            current.tab_id == next.tab_id
                && current.kind == next.kind
                && current.rule_index == next.rule_index
                && current.sort_index == next.sort_index
        });
        self.data_filter_popover = if is_same_popover {
            None
        } else {
            self.local_filter_popover = None;
            self.local_filter_manager_popover = None;
            if matches!(
                kind,
                DataFilterPopoverKind::Field
                    | DataFilterPopoverKind::SortField
                    | DataFilterPopoverKind::Value
            ) {
                self.data_filter_value_search.clear();
                self.data_filter_search_input.update(cx, |input, cx| {
                    input.set_value(String::new(), window, cx);
                });
            }
            if kind == DataFilterPopoverKind::Value {
                self.init_data_filter_value_draft(tab_id, rule_index.unwrap_or(0), window, cx);
            } else {
                // 非「值」弹层打开时丢弃值编辑草稿，避免下次误用。
                self.data_filter_value_draft = None;
            }
            Some(next)
        };
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    /// 打开值编辑器时，从当前条件快照初始化独立草稿。
    /// 后续所有值编辑（手动添加、建议值勾选、批量粘贴）只改草稿，
    /// 「确定」才把草稿写回条件，取消/关闭/Esc/外部点击都丢弃草稿。
    fn init_data_filter_value_draft(
        &mut self,
        tab_id: TabId,
        rule_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (draft, input_text) = self
            .data_filter_draft_rules
            .get(&tab_id)
            .and_then(|rules| rules.get(rule_index))
            .map(|rule| {
                let draft = DataFilterValueDraft {
                    tab_id,
                    rule_index,
                    field: rule.field.clone().unwrap_or_default(),
                    operator: rule.operator,
                    values: rule.values.clone(),
                    input: String::new(),
                    batch_open: false,
                    batch_text: String::new(),
                    batch_separator: BatchSeparator::Newline,
                };
                // 单值弹层的输入框就是那一个值，打开时直接带上当前值；
                // 多值弹层的输入框是「新增一个值」的入口，必须为空。
                let input_text = if rule.operator.value_arity() == DataFilterValueArity::Single {
                    rule.values.first().cloned().unwrap_or_default()
                } else {
                    String::new()
                };
                (draft, input_text)
            })
            .unwrap_or_else(|| {
                (
                    DataFilterValueDraft {
                        tab_id,
                        rule_index,
                        field: String::new(),
                        operator: DataFilterOperator::InList,
                        values: Vec::new(),
                        input: String::new(),
                        batch_open: false,
                        batch_text: String::new(),
                        batch_separator: BatchSeparator::Newline,
                    },
                    String::new(),
                )
            });
        self.data_filter_value_draft = Some(draft);
        self.data_filter_value_input_text = input_text.clone();
        self.data_filter_value_input.update(cx, |input, cx| {
            input.set_value(input_text, window, cx);
        });
        self.data_filter_batch_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
    }

    /// 当前字段在数据页列定义中的类型名；页面未加载或缺列时返回 None（按文本处理）。
    fn data_filter_column_type_name(&self, tab_id: TabId, field: &str) -> Option<String> {
        self.controller
            .state()
            .tabs
            .iter()
            .find_map(|tab| match &tab.kind {
                TabKind::DataEditor(editor) if tab.id == tab_id => editor.page.as_ref(),
                _ => None,
            })
            .and_then(|page| page.columns.iter().find(|column| column.name == field))
            .and_then(|column| column.type_name.clone())
    }

    fn select_data_filter_field(
        &mut self,
        tab_id: TabId,
        rule_index: usize,
        field: String,
        cx: &mut Context<Self>,
    ) {
        // 先取新字段类型：旧值能否保留取决于目标列类型。
        let new_type_name = self.data_filter_column_type_name(tab_id, &field);
        let mut message = None;
        if let Some(rule) = self
            .data_filter_draft_rules
            .get_mut(&tab_id)
            .and_then(|rules| rules.get_mut(rule_index))
        {
            let (new_rule, kept, dropped) =
                data_filter_rule_after_field_switch(field, rule.clone(), new_type_name.as_deref());
            *rule = new_rule;
            if dropped > 0 {
                message = Some(if kept > 0 {
                    tr("app.filter.toast.field_changed_partial")
                        .replace("%{count}", &dropped.to_string())
                } else {
                    tr("app.filter.toast.field_changed").to_string()
                });
            }
        }
        if let Some(message) = message {
            self.show_message(message, AppMessageKind::Info, cx);
        }
        // 同时丢弃值草稿；再次打开值编辑器时（toggle_data_filter_popover）会
        // 重新清空输入与搜索状态，新字段只能显示和选择自己的建议值。
        self.data_filter_value_draft = None;
        self.data_filter_value_input_text = String::new();
        self.data_filter_value_search.clear();
        self.data_filter_popover = None;
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    fn apply_data_filter_and_sort(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self.request_dirty_data_action(PendingDirtyDataAction::ApplyFilterSort(tab_id), cx) {
            return;
        }
        self.perform_data_filter_and_sort(tab_id, cx);
    }

    fn perform_data_filter_and_sort(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        // 用户重新应用筛选时接管正在进行的 AI 页面查询：新令牌会让 AI 迟到结果失效。
        if self.ai_chat.table_page_task.is_some() {
            self.take_over_ai_table_page(tab_id, cx);
        }
        self.data_filter_applying_tabs.insert(tab_id);
        let mode = self
            .data_filter_modes
            .get(&tab_id)
            .copied()
            .unwrap_or(DataFilterMode::Builder);
        let filter_rules = match mode {
            DataFilterMode::Builder => {
                self.sync_data_filter_builder_to_text(tab_id);
                self.data_filter_draft_rules
                    .get(&tab_id)
                    .cloned()
                    .unwrap_or_default()
            }
            DataFilterMode::Text => {
                if let Err(error) = self.data_filter_expression_for_tab(tab_id) {
                    self.data_filter_applying_tabs.remove(&tab_id);
                    self.show_message(error.to_string(), AppMessageKind::Warning, cx);
                    cx.notify();
                    return;
                }
                // 输入模式以结构化表达式为准；这里的规则只用于 IDE 中“当前页面”
                // 上下文和草稿同步，真实执行走 `data_filter_expression_for_tab`。
                self.data_filter_rules_from_text(tab_id)
                    .unwrap_or_default()
            }
        };
        if mode == DataFilterMode::Text {
            // 输入模式执行时也同步表单草稿，确保之后切回表单看到的就是本次执行条件。
            if let Err(error) = self.sync_data_filter_text_to_builder(tab_id) {
                self.data_filter_applying_tabs.remove(&tab_id);
                self.show_message(error, AppMessageKind::Warning, cx);
                cx.notify();
                return;
            }
        }

        match mode {
            DataFilterMode::Builder => {
                if filter_rules.is_empty() {
                    self.data_filter_rules.remove(&tab_id);
                } else {
                    self.data_filter_rules.insert(tab_id, filter_rules);
                }
            }
            DataFilterMode::Text => {
                self.data_filter_rules.remove(&tab_id);
            }
        }

        if let Some(rules) = self.data_sort_draft_rules.get(&tab_id).cloned() {
            if rules.is_empty() {
                self.data_sort_rules.remove(&tab_id);
            } else {
                self.data_sort_rules.insert(tab_id, rules);
            }
        } else {
            self.data_sort_rules.remove(&tab_id);
        }

        if let Some(limit) =
            parse_data_editor_sql_text(&self.data_sql_panel_input.read(cx).value().to_string())
                .and_then(|parsed| parsed.limit)
        {
            let effective_limit = data_editor_effective_limit(limit);
            let offset = self
                .controller
                .state()
                .tabs
                .iter()
                .find_map(|tab| match &tab.kind {
                    TabKind::DataEditor(editor) if tab.id == tab_id => {
                        Some(editor.pagination.offset)
                    }
                    _ => None,
                })
                .unwrap_or(0);
            let _ = self.controller.dispatch(AppCommand::SetDataPagePagination {
                tab_id,
                offset,
                limit: effective_limit,
            });
            // 只在「超出封顶」时提示。下限（0 → 1）静默处理，否则会弹出
            // 「最大支持 1000，已自动改为 1000」这种对不上号的文案。
            let max = data_table_page_size_max();
            if limit > max {
                self.show_message(
                    tr("app.filter.error.limit_max").replace("%{max}", &max.to_string()),
                    AppMessageKind::Warning,
                    cx,
                );
            }
        }

        self.data_filter_popover = None;
        self.sync_table_hover_overlay_block(cx);
        self.start_data_filter_sort_load(tab_id, cx);
        cx.notify();
    }

    fn select_data_filter_operator(
        &mut self,
        tab_id: TabId,
        rule_index: usize,
        operator: DataFilterOperator,
        cx: &mut Context<Self>,
    ) {
        if let Some(rule) = self
            .data_filter_draft_rules
            .get_mut(&tab_id)
            .and_then(|rules| rules.get_mut(rule_index))
        {
            // IN ↔ NOT IN 可保留已有列表；切到单值/区间/无值运算符时清空旧列表，
            // 避免旧值以错误的 OR 组合或多余的 BETWEEN 参数参与查询。
            let old_rule = rule.clone();
            *rule = data_filter_rule_after_operator_switch(operator, old_rule);
            // 值弹层若正编辑已切换的运算符，同步更新其快照与草稿。
            if let Some(draft) = self.data_filter_value_draft.as_mut()
                && draft.tab_id == tab_id
                && draft.rule_index == rule_index
            {
                draft.operator = operator;
                draft.values = data_filter_values_for_arity(&draft.values, operator);
            }
        }
        self.data_filter_popover = None;
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    fn toggle_data_filter_combinator(
        &mut self,
        tab_id: TabId,
        rule_index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(rule) = self
            .data_filter_draft_rules
            .get_mut(&tab_id)
            .and_then(|rules| rules.get_mut(rule_index))
        {
            rule.connector = match rule.connector {
                fluxdb_core::FilterCombinator::And => fluxdb_core::FilterCombinator::Or,
                fluxdb_core::FilterCombinator::Or => fluxdb_core::FilterCombinator::And,
            };
        }
        self.data_filter_popover = None;
        self.sync_data_filter_builder_to_text(tab_id);
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    /// 单值弹层勾选建议值：把该值填进输入框；再点同一项取消勾选（清空输入）。
    ///
    /// 单值形态下「勾选」不是多选，而是指定那一个值，因此直接改草稿输入，
    /// 由「确定」统一写回条件。
    fn pick_data_filter_single_value(
        &mut self,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = self.data_filter_value_draft.as_mut() else {
            return;
        };
        let next = if draft.input == value {
            String::new()
        } else {
            value
        };
        draft.input = next.clone();
        self.data_filter_value_input_text = next.clone();
        self.data_filter_value_input.update(cx, |input, cx| {
            input.set_value(next, window, cx);
        });
        cx.notify();
    }

    /// 值弹层输入框回车：单值形态直接「确定」写回条件；多值形态追加为新值，保持连续输入。
    fn commit_data_filter_value_input(&mut self, value: String, cx: &mut Context<Self>) {
        self.update_data_filter_manual_value(value, cx);
        let draft = self.data_filter_value_draft.as_ref();
        let single = draft.is_some_and(|draft| {
            draft.operator.value_arity() == DataFilterValueArity::Single
        });
        let tab_id = draft.map(|draft| draft.tab_id);
        if single && let Some(tab_id) = tab_id {
            self.apply_data_filter_value_draft(tab_id, cx);
        } else {
            self.add_data_filter_manual_value(TabId(0), cx);
        }
    }

    /// 建议值勾选：切换草稿中的某个值（已选→取消，未选→加入），不直接改条件。
    fn toggle_data_filter_value(
        &mut self,
        tab_id: TabId,
        _rule_index: usize,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(draft) = self.data_filter_value_draft.as_mut()
            && draft.tab_id == tab_id
        {
            if let Some(index) = draft.values.iter().position(|selected| selected == &value) {
                draft.values.remove(index);
            } else {
                if !data_filter_rule_can_add_more_value(draft.operator, draft.values.len()) {
                    self.show_message(
                        tr("app.filter.error.between_max").to_string(),
                        AppMessageKind::Warning,
                        cx,
                    );
                } else {
                    draft.values.push(value);
                }
            }
        }
        cx.notify();
    }

    /// 删除草稿中的单个已选值（标签上的删除按钮）。
    fn remove_data_filter_value(
        &mut self,
        tab_id: TabId,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(draft) = self.data_filter_value_draft.as_mut()
            && draft.tab_id == tab_id
        {
            if let Some(index) = draft.values.iter().position(|selected| selected == &value) {
                draft.values.remove(index);
            }
        }
        cx.notify();
    }

    /// 清空草稿已选值（「清空」按钮）。
    fn clear_data_filter_values(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(draft) = self.data_filter_value_draft.as_mut()
            && draft.tab_id == tab_id
        {
            draft.values.clear();
        }
        cx.notify();
    }

    /// 批量粘贴区「添加到已选」：按分隔方式解析并去重后合入草稿。
    fn add_data_filter_batch_values(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        // 先解析出待添加值，随后释放草稿借用再调用 show_message，避免双重可变借用。
        let (added_values, duplicated, ignored_empty) = {
            let Some(draft) = self.data_filter_value_draft.as_mut() else {
                return;
            };
            if draft.tab_id != tab_id {
                return;
            }
            filter_batch_values_to_add(&draft.batch_text, draft.batch_separator, &draft.values)
        };

        if !added_values.is_empty() {
            if let Some(draft) = self.data_filter_value_draft.as_mut()
                && draft.tab_id == tab_id
            {
                for value in added_values.iter().cloned() {
                    if !data_filter_rule_can_add_more_value(draft.operator, draft.values.len()) {
                        break;
                    }
                    draft.values.push(value);
                }
                draft.batch_text.clear();
            }
            self.show_message(
                tr("app.filter.toast.values_added")
                    .replace("%{added}", &added_values.len().to_string())
                    .replace("%{duplicated}", &duplicated.to_string())
                    .replace(
                        "%{ignored}",
                        &if ignored_empty > 0 {
                            tr("app.filter.toast.ignored_empty")
                                .replace("%{count}", &ignored_empty.to_string())
                        } else {
                            String::new()
                        },
                    ),
                AppMessageKind::Info,
                cx,
            );
        }
        cx.notify();
    }

    /// 展开/收起批量粘贴区。
    fn toggle_data_filter_batch(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(draft) = self.data_filter_value_draft.as_mut()
            && draft.tab_id == tab_id
        {
            draft.batch_open = !draft.batch_open;
        }
        cx.notify();
    }

    /// 切换批量粘贴分隔方式。
    fn set_data_filter_batch_separator(
        &mut self,
        tab_id: TabId,
        separator: BatchSeparator,
        cx: &mut Context<Self>,
    ) {
        if let Some(draft) = self.data_filter_value_draft.as_mut()
            && draft.tab_id == tab_id
        {
            draft.batch_separator = separator;
        }
        cx.notify();
    }

    /// 取消值编辑：丢弃草稿并关闭弹层（取消/关闭/Esc/外部点击共用）。
    fn cancel_data_filter_value(&mut self, cx: &mut Context<Self>) {
        self.data_filter_value_draft = None;
        self.data_filter_popover = None;
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    /// 单值手动添加：Enter 或「添加」提交当前输入框内容到草稿，成功后清空并保留焦点。
    fn add_data_filter_manual_value(&mut self, _tab_id: TabId, cx: &mut Context<Self>) {
        let Some(draft) = self.data_filter_value_draft.as_mut() else {
            return;
        };
        // 同一时间只有唯一值弹层在编辑，草稿自带的 tab_id/rule_index 即目标条件。
        let raw = std::mem::take(&mut draft.input);
        // 空输入不得自动成为空字符串值；保留字符串内有意义的空格/引号/逗号。
        if raw.trim().is_empty() {
            cx.notify();
            return;
        }
        let value = raw.trim().to_string();
        if draft.values.iter().any(|selected| selected == &value) {
            self.show_message(tr("app.filter.error.value_selected").as_ref(), AppMessageKind::Warning, cx);
        } else if !data_filter_rule_can_add_more_value(draft.operator, draft.values.len()) {
            self.show_message(
                tr("app.filter.error.between_max").to_string(),
                AppMessageKind::Warning,
                cx,
            );
        } else {
            draft.values.push(value);
        }
        // 保持输入框空并保留焦点，方便连续输入。
        self.data_filter_value_input_text.clear();
        cx.notify();
    }

    /// 「确定」：把草稿写回条件并关闭弹层。
    ///
    /// 若单值输入仍有未提交内容，先校验并入草稿；若批量粘贴区仍有未处理内容，
    /// 提示先添加或清空，不悄悄丢弃。
    fn apply_data_filter_value_draft(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(mut draft) = self.data_filter_value_draft.take() else {
            self.data_filter_popover = None;
            self.sync_table_hover_overlay_block(cx);
            cx.notify();
            return;
        };
        if draft.tab_id != tab_id {
            self.data_filter_popover = None;
            self.sync_table_hover_overlay_block(cx);
            cx.notify();
            return;
        }

        // 单值形态：输入框内容就是那一个值（空输入即清空该值），
        // 没有已选列表/批量区的中间态需要校验。
        if draft.operator.value_arity() == DataFilterValueArity::Single {
            let value = std::mem::take(&mut draft.input).trim().to_string();
            draft.values = if value.is_empty() {
                Vec::new()
            } else {
                vec![value]
            };
        } else {
            // 未提交的单值输入先纳入草稿（沿用与手动添加相同的去空/去重规则）。
            let pending = std::mem::take(&mut draft.input);
            if !pending.trim().is_empty()
                && data_filter_rule_can_add_more_value(draft.operator, draft.values.len())
            {
                draft.values.push(pending.trim().to_string());
            } else if !pending.trim().is_empty() {
                // 介于/不介于已满两个值：未提交的单值无法加入，明确提示而非静默丢弃。
                self.show_message(
                    tr("app.filter.error.between_max").to_string(),
                    AppMessageKind::Warning,
                    cx,
                );
            }
            // 批量区若有未处理内容，提示后返回，不静默丢失。
            if !draft.batch_text.trim().is_empty() {
                self.show_message(
                    tr("app.filter.error.pending_batch").to_string(),
                    AppMessageKind::Warning,
                    cx,
                );
                self.data_filter_value_draft = Some(draft);
                cx.notify();
                return;
            }

            // 空列表校验：IN / NOT IN 不允许空列表，避免静默退化为无条件查询。
            if matches!(
                draft.operator,
                DataFilterOperator::InList | DataFilterOperator::NotInList
            ) && draft.values.is_empty()
            {
                self.show_message(
                    tr("app.filter.error.list_empty").to_string(),
                    AppMessageKind::Warning,
                    cx,
                );
                self.data_filter_value_draft = Some(draft);
                cx.notify();
                return;
            }
        }

        // 只更新当前条件，不触发查询；仍由「应用筛选 & 排序」执行查询。
        if let Some(rule) = self
            .data_filter_draft_rules
            .get_mut(&tab_id)
            .and_then(|rules| rules.get_mut(draft.rule_index))
        {
            let values_unchanged =
                rule.operator == draft.operator && rule.values == draft.values;
            rule.values = draft.values;
            if !values_unchanged {
                rule.typed_values = None;
            }
        }
        self.data_filter_popover = None;
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    fn add_data_sort_rule(
        &mut self,
        tab_id: TabId,
        default_field: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = default_field else {
            return;
        };
        self.data_sort_clear_undo.remove(&tab_id);
        let rules = self.data_sort_draft_rules.entry(tab_id).or_default();
        rules.push(DataSortRule {
            enabled: true,
            field,
            ascending: true,
        });
        self.data_filter_popover = None;
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    fn select_data_sort_field(
        &mut self,
        tab_id: TabId,
        sort_index: usize,
        field: String,
        cx: &mut Context<Self>,
    ) {
        self.data_sort_clear_undo.remove(&tab_id);
        let rules = self.data_sort_draft_rules.entry(tab_id).or_default();
        if sort_index >= rules.len() {
            rules.push(DataSortRule {
                enabled: true,
                field,
                ascending: true,
            });
        } else if let Some(rule) = rules.get_mut(sort_index) {
            rule.field = field;
        }
        self.data_filter_popover = None;
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    fn toggle_data_sort_direction(
        &mut self,
        tab_id: TabId,
        sort_index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(rule) = self
            .data_sort_draft_rules
            .get_mut(&tab_id)
            .and_then(|rules| rules.get_mut(sort_index))
        {
            self.data_sort_clear_undo.remove(&tab_id);
            rule.ascending = !rule.ascending;
            cx.notify();
        }
    }

    fn delete_data_sort_rule(&mut self, tab_id: TabId, sort_index: usize, cx: &mut Context<Self>) {
        self.data_sort_clear_undo.remove(&tab_id);
        if let Some(rules) = self.data_sort_draft_rules.get_mut(&tab_id) {
            if sort_index < rules.len() {
                rules.remove(sort_index);
            }
            if rules.is_empty() {
                self.data_sort_draft_rules.remove(&tab_id);
            }
        }
        if self
            .data_filter_popover
            .is_some_and(|popover| popover.tab_id == tab_id)
        {
            self.data_filter_popover = None;
            self.sync_table_hover_overlay_block(cx);
        }
        cx.notify();
    }

    fn move_data_sort_rule(
        &mut self,
        tab_id: TabId,
        from: usize,
        to: usize,
        cx: &mut Context<Self>,
    ) {
        if from == to {
            return;
        }
        if let Some(rules) = self.data_sort_draft_rules.get_mut(&tab_id)
            && from < rules.len()
            && to < rules.len()
        {
            let rule = rules.remove(from);
            rules.insert(to, rule);
            self.data_sort_clear_undo.remove(&tab_id);
            self.sync_data_filter_builder_to_text(tab_id);
            cx.notify();
        }
    }

    fn clear_data_sort_rules(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(previous) = self.data_sort_draft_rules.remove(&tab_id) else {
            return;
        };
        self.data_sort_clear_undo.insert(tab_id, previous);
        self.sync_data_filter_builder_to_text(tab_id);
        self.data_filter_popover = None;
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    fn undo_clear_data_sort_rules(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(previous) = self.data_sort_clear_undo.remove(&tab_id) else {
            return;
        };
        self.data_sort_draft_rules.insert(tab_id, previous);
        self.sync_data_filter_builder_to_text(tab_id);
        cx.notify();
    }

    fn clear_data_filter_and_sort_rules(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.data_filter_draft_rules.remove(&tab_id);
        self.data_sort_draft_rules.remove(&tab_id);
        self.data_sort_clear_undo.remove(&tab_id);
        self.data_filter_rules.remove(&tab_id);
        self.data_sort_rules.remove(&tab_id);
        self.data_filter_grouped_tabs.remove(&tab_id);
        self.data_filter_texts.remove(&tab_id);
        self.data_filter_condition_completion_signatures.remove(&tab_id);
        self.data_sort_texts.remove(&tab_id);
        if self
            .data_filter_popover
            .is_some_and(|popover| popover.tab_id == tab_id)
        {
            self.data_filter_popover = None;
            self.sync_table_hover_overlay_block(cx);
        }
        cx.notify();
    }

    fn toggle_data_filter_panel(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.data_filter_panels.contains(&tab_id) {
            self.data_filter_panels.remove(&tab_id);
            self.data_filter_condition_completion_signatures.remove(&tab_id);
            if self
                .data_filter_popover
                .is_some_and(|popover| popover.tab_id == tab_id)
            {
                self.data_filter_popover = None;
            }
        } else {
            self.data_filter_panels.insert(tab_id);
            if !self.data_filter_draft_rules.contains_key(&tab_id)
                && !self.data_filter_texts.contains_key(&tab_id)
                && let Some(rule) = self.data_filter_rules.get(&tab_id).cloned()
            {
                self.data_filter_draft_rules.insert(tab_id, rule);
            }
            if !self.data_sort_draft_rules.contains_key(&tab_id)
                && !self.data_sort_texts.contains_key(&tab_id)
                && let Some(rule) = self.data_sort_rules.get(&tab_id).cloned()
            {
                self.data_sort_draft_rules.insert(tab_id, rule);
            }
            self.sync_data_filter_text_inputs(tab_id, window, cx);
        }
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

}
