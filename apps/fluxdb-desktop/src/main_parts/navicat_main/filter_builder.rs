impl NavicatMain {
    fn add_data_filter_rule(
        &mut self,
        tab_id: TabId,
        default_field: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.insert_data_filter_rule(tab_id, None, default_field, false, cx);
    }

    fn add_data_filter_rule_after(
        &mut self,
        tab_id: TabId,
        rule_index: usize,
        default_field: Option<String>,
        grouped: bool,
        cx: &mut Context<Self>,
    ) {
        self.insert_data_filter_rule(tab_id, Some(rule_index + 1), default_field, grouped, cx);
    }

    fn insert_data_filter_rule(
        &mut self,
        tab_id: TabId,
        insert_at: Option<usize>,
        default_field: Option<String>,
        grouped: bool,
        cx: &mut Context<Self>,
    ) {
        let mut rule = DataFilterRule::default();
        rule.field = default_field;
        rule.grouped = grouped;
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
        self.insert_data_filter_rule(tab_id, None, default_field, true, cx);
    }

    fn toggle_data_filter_popover(
        &mut self,
        tab_id: TabId,
        rule_index: Option<usize>,
        kind: DataFilterPopoverKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = DataFilterPopover {
            tab_id,
            kind,
            rule_index,
            sort_index: None,
        };
        self.data_filter_popover = if self.data_filter_popover == Some(next) {
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
                let manual_value = self
                    .data_filter_draft_rules
                    .get(&tab_id)
                    .and_then(|rules| rules.get(rule_index.unwrap_or(0)))
                    .and_then(|rule| rule.values.iter().next().cloned())
                    .unwrap_or_default();
                self.data_filter_value_input_text = manual_value.clone();
                self.data_filter_value_input.update(cx, |input, cx| {
                    input.set_value(manual_value, window, cx);
                });
            }
            Some(next)
        };
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    fn select_data_filter_field(
        &mut self,
        tab_id: TabId,
        rule_index: usize,
        field: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(rule) = self
            .data_filter_draft_rules
            .get_mut(&tab_id)
            .and_then(|rules| rules.get_mut(rule_index))
        {
            rule.field = Some(field);
        }
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
        if self.data_filter_applying_tabs.contains(&tab_id) {
            return;
        }

        self.data_filter_applying_tabs.insert(tab_id);
        match self
            .data_filter_modes
            .get(&tab_id)
            .copied()
            .unwrap_or(DataFilterMode::Builder)
        {
            DataFilterMode::Builder => self.sync_data_filter_builder_to_text(tab_id),
            DataFilterMode::Text => self.sync_data_filter_text_to_builder(tab_id),
        }

        if let Some(rules) = self.data_filter_draft_rules.get(&tab_id).cloned() {
            if rules.is_empty() {
                self.data_filter_rules.remove(&tab_id);
            } else {
                self.data_filter_rules.insert(tab_id, rules);
            }
        } else {
            self.data_filter_rules.remove(&tab_id);
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
            let effective_limit = limit.clamp(1, 100);
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
            if limit > 100 {
                self.show_message(
                    "LIMIT 最大支持 100，已自动改为 100",
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
            rule.operator = operator;
        }
        self.data_filter_popover = None;
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    fn toggle_data_filter_value(
        &mut self,
        tab_id: TabId,
        rule_index: usize,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(rule) = self
            .data_filter_draft_rules
            .get_mut(&tab_id)
            .and_then(|rules| rules.get_mut(rule_index))
        {
            if !rule.values.remove(&value) {
                rule.values.insert(value);
            }
            self.data_filter_value_input_text =
                rule.values.iter().next().cloned().unwrap_or_default();
        }
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
            rule.ascending = !rule.ascending;
            cx.notify();
        }
    }

    fn set_data_sort_direction(
        &mut self,
        tab_id: TabId,
        sort_index: usize,
        ascending: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(rule) = self
            .data_sort_draft_rules
            .get_mut(&tab_id)
            .and_then(|rules| rules.get_mut(sort_index))
        {
            rule.ascending = ascending;
        }
        self.data_filter_popover = None;
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    fn open_data_sort_field_menu(
        &mut self,
        tab_id: TabId,
        sort_index: usize,
        cx: &mut Context<Self>,
    ) {
        self.data_filter_popover = Some(DataFilterPopover {
            tab_id,
            kind: DataFilterPopoverKind::SortField,
            rule_index: None,
            sort_index: Some(sort_index),
        });
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

    fn delete_data_sort_rule(&mut self, tab_id: TabId, sort_index: usize, cx: &mut Context<Self>) {
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

    fn clear_data_filter_and_sort_rules(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.data_filter_draft_rules.remove(&tab_id);
        self.data_sort_draft_rules.remove(&tab_id);
        self.data_filter_rules.remove(&tab_id);
        self.data_sort_rules.remove(&tab_id);
        self.data_filter_grouped_tabs.remove(&tab_id);
        self.data_filter_texts.remove(&tab_id);
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
            self.data_filter_draft_rules.remove(&tab_id);
            self.data_sort_draft_rules.remove(&tab_id);
            self.data_filter_grouped_tabs.remove(&tab_id);
            self.data_filter_modes.remove(&tab_id);
            self.data_filter_texts.remove(&tab_id);
            self.data_sort_texts.remove(&tab_id);
            if self
                .data_filter_popover
                .is_some_and(|popover| popover.tab_id == tab_id)
            {
                self.data_filter_popover = None;
            }
        } else {
            self.data_filter_panels.insert(tab_id);
            if let Some(rule) = self.data_filter_rules.get(&tab_id).cloned() {
                self.data_filter_draft_rules.insert(tab_id, rule);
            } else {
                self.data_filter_draft_rules.remove(&tab_id);
            }
            if let Some(rule) = self.data_sort_rules.get(&tab_id).cloned() {
                self.data_sort_draft_rules.insert(tab_id, rule);
            } else {
                self.data_sort_draft_rules.remove(&tab_id);
            }
            self.sync_data_filter_text_inputs(tab_id, window, cx);
        }
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

}
