impl NavicatMain {
    fn update_data_filter_text(&mut self, value: String, cx: &mut Context<Self>) {
        if let Some(tab_id) = self.active_data_filter_tab_id() {
            if value.trim().is_empty() {
                self.data_filter_texts.remove(&tab_id);
            } else {
                self.data_filter_texts.insert(tab_id, value);
            }
        }
        cx.notify();
    }

    fn update_data_sort_text(&mut self, value: String, cx: &mut Context<Self>) {
        if let Some(tab_id) = self.active_data_filter_tab_id() {
            if value.trim().is_empty() {
                self.data_sort_texts.remove(&tab_id);
            } else {
                self.data_sort_texts.insert(tab_id, value);
            }
        }
        cx.notify();
    }

    fn update_data_editor_sql_text(&mut self, value: String, cx: &mut Context<Self>) {
        let Some(tab_id) = self.active_data_filter_tab_id() else {
            cx.notify();
            return;
        };
        let Some(parsed) = parse_data_editor_sql_text(&value) else {
            cx.notify();
            return;
        };

        if parsed.filter_text.is_empty() {
            self.data_filter_texts.remove(&tab_id);
            self.data_filter_draft_rules.remove(&tab_id);
        } else {
            if let Some(rules) = parse_data_filter_rules_text(&parsed.filter_text) {
                self.data_filter_draft_rules.insert(tab_id, rules);
            }
            self.data_filter_texts.insert(tab_id, parsed.filter_text);
        }

        if parsed.sort_text.is_empty() {
            self.data_sort_texts.remove(&tab_id);
            self.data_sort_draft_rules.remove(&tab_id);
        } else {
            if let Some(rules) = parse_data_sort_rules_text(&parsed.sort_text) {
                self.data_sort_draft_rules.insert(tab_id, rules);
            }
            self.data_sort_texts.insert(tab_id, parsed.sort_text);
        }
        cx.notify();
    }

    fn start_footer_sql_selection(
        &mut self,
        text: &str,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let offset = sql_text_selection_offset(
            text,
            self.data_sql_footer_selection.bounds.as_ref(),
            position,
        );
        self.data_sql_footer_selection.anchor = offset;
        self.data_sql_footer_selection.cursor = offset;
        self.data_sql_footer_selection.selecting = true;
        cx.notify();
    }

    fn select_all_footer_sql(&mut self, text: &str, cx: &mut Context<Self>) {
        self.data_sql_footer_selection.anchor = 0;
        self.data_sql_footer_selection.cursor = text.len();
        self.data_sql_footer_selection.selecting = false;
        cx.notify();
    }

    fn update_footer_sql_selection(
        &mut self,
        text: &str,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !self.data_sql_footer_selection.selecting {
            return;
        }
        self.data_sql_footer_selection.cursor = sql_text_selection_offset(
            text,
            self.data_sql_footer_selection.bounds.as_ref(),
            position,
        );
        cx.notify();
    }

    fn finish_footer_sql_selection(&mut self, cx: &mut Context<Self>) {
        self.data_sql_footer_selection.selecting = false;
        cx.notify();
    }

    fn sync_data_filter_builder_to_text(&mut self, tab_id: TabId) {
        let text = self
            .data_filter_draft_rules
            .get(&tab_id)
            .map(|rules| data_filter_rules_sql_pretty(rules))
            .unwrap_or_default();
        if text.is_empty() {
            self.data_filter_texts.remove(&tab_id);
        } else {
            self.data_filter_texts.insert(tab_id, text);
        }

        if let Some(sort_text) = self
            .data_sort_draft_rules
            .get(&tab_id)
            .map(|rules| data_sort_rules_text(rules))
            .filter(|text| !text.is_empty())
        {
            self.data_sort_texts.insert(tab_id, sort_text);
        } else {
            self.data_sort_texts.remove(&tab_id);
        }
    }

    fn sync_data_filter_text_to_builder(&mut self, tab_id: TabId) {
        if let Some(rules) = self
            .data_filter_texts
            .get(&tab_id)
            .and_then(|text| parse_data_filter_rules_text(text))
        {
            if rules.is_empty() {
                self.data_filter_draft_rules.remove(&tab_id);
            } else {
                self.data_filter_draft_rules.insert(tab_id, rules);
            }
        }

        if let Some(rules) = self
            .data_sort_texts
            .get(&tab_id)
            .and_then(|text| parse_data_sort_rules_text(text))
        {
            if rules.is_empty() {
                self.data_sort_draft_rules.remove(&tab_id);
            } else {
                self.data_sort_draft_rules.insert(tab_id, rules);
            }
        }
    }

    fn sync_data_filter_text_inputs(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let filter_text = self
            .data_filter_texts
            .get(&tab_id)
            .cloned()
            .unwrap_or_default();
        if self.data_filter_text_input.read(cx).value().to_string() != filter_text {
            self.data_filter_text_input.update(cx, |input, cx| {
                input.set_value(filter_text, window, cx);
            });
        }

        let sort_text = self
            .data_sort_texts
            .get(&tab_id)
            .cloned()
            .unwrap_or_default();
        if self.data_sort_text_input.read(cx).value().to_string() != sort_text {
            self.data_sort_text_input.update(cx, |input, cx| {
                input.set_value(sort_text, window, cx);
            });
        }
    }

}
