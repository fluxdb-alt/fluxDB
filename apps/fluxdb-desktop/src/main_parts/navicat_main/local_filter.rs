impl NavicatMain {
    fn update_data_filter_value_search(&mut self, query: String, cx: &mut Context<Self>) {
        self.data_filter_value_search = query;
        let until = Instant::now() + Duration::from_millis(180);
        self.data_filter_value_search_loading_until = Some(until);
        self.data_filter_value_search_task = Some(cx.spawn(async move |view, cx| {
            smol::Timer::after(Duration::from_millis(190)).await;
            view.update(cx, move |this, cx| {
                if this
                    .data_filter_value_search_loading_until
                    .is_some_and(|current_until| current_until <= until)
                {
                    this.data_filter_value_search_loading_until = None;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }

    fn select_local_filter_manager_field(
        &mut self,
        field: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.local_filter_manager_field = Some(field);
        self.local_filter_value.clear();
        self.local_filter_search.clear();
        self.local_filter_manager_field_open = false;
        self.local_filter_manager_values_open = true;
        self.local_filter_search_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        cx.notify();
    }

    fn toggle_local_filter_manager_fields(&mut self, cx: &mut Context<Self>) {
        self.local_filter_manager_field_open = !self.local_filter_manager_field_open;
        if self.local_filter_manager_field_open {
            self.local_filter_manager_values_open = false;
        }
        cx.notify();
    }

    fn start_local_filter_manager_condition(
        &mut self,
        field_names: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let field = field_names
            .iter()
            .find(|field| {
                self.local_filter_manager_draft_filters
                    .get(field.as_str())
                    .is_none_or(BTreeSet::is_empty)
            })
            .or_else(|| field_names.first())
            .cloned();
        if let Some(field) = field {
            self.local_filter_manager_field = Some(field);
        }
        self.local_filter_manager_field_open = true;
        self.local_filter_manager_values_open = false;
        cx.notify();
    }

    fn toggle_local_filter_manager_values(&mut self, cx: &mut Context<Self>) {
        self.local_filter_manager_values_open = !self.local_filter_manager_values_open;
        if self.local_filter_manager_values_open {
            self.local_filter_manager_field_open = false;
        }
        cx.notify();
    }

    fn open_local_filter_manager_values_for_field(
        &mut self,
        field: String,
        cx: &mut Context<Self>,
    ) {
        self.local_filter_manager_field = Some(field);
        self.local_filter_manager_field_open = false;
        self.local_filter_manager_values_open = true;
        cx.notify();
    }

    fn close_local_filter_manager_dropdowns(&mut self, cx: &mut Context<Self>) {
        let changed = self.local_filter_manager_field_open || self.local_filter_manager_values_open;
        self.local_filter_manager_field_open = false;
        self.local_filter_manager_values_open = false;
        if changed {
            cx.notify();
        }
    }

    fn toggle_local_filter_manager_value(
        &mut self,
        field: String,
        value: String,
        cx: &mut Context<Self>,
    ) {
        self.local_filter_manager_draft_filters = local_table_filters_after_value_toggle(
            std::mem::take(&mut self.local_filter_manager_draft_filters),
            field.as_str(),
            value.as_str(),
        );
        cx.notify();
    }

    fn remove_local_filter_manager_field(&mut self, field: String, cx: &mut Context<Self>) {
        self.local_filter_manager_draft_filters.remove(&field);
        cx.notify();
    }

    fn reset_local_filter_manager(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.local_filter_manager_draft_filters = self
            .local_table_filters
            .get(&tab_id)
            .cloned()
            .unwrap_or_default();
        cx.notify();
    }

    fn apply_local_filter_manager(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let filters = self
            .local_filter_manager_draft_filters
            .iter()
            .filter(|(_, values)| !values.is_empty())
            .map(|(field, values)| (field.clone(), values.clone()))
            .collect::<BTreeMap<_, _>>();

        if filters.is_empty() {
            self.local_table_filters.remove(&tab_id);
        } else {
            self.local_table_filters.insert(tab_id, filters);
        }
        self.refresh_active_data_table(tab_id, cx);
        self.sync_table_hover_overlay_block(cx);
    }

    fn clear_local_filter_manager(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.local_table_filters.remove(&tab_id);
        self.local_filter_manager_draft_filters.clear();
        self.local_filter_manager_popover = None;
        self.refresh_active_data_table(tab_id, cx);
        self.sync_table_hover_overlay_block(cx);
    }

    fn update_local_filter_value(&mut self, value: String, cx: &mut Context<Self>) {
        self.local_filter_value = value;
        cx.notify();
    }

    fn update_local_filter_search(&mut self, value: String, cx: &mut Context<Self>) {
        self.local_filter_search = value;
        cx.notify();
    }

    fn toggle_local_filter_value(&mut self, value: String, cx: &mut Context<Self>) {
        if !self.local_filter_draft_values.remove(&value) {
            self.local_filter_draft_values.insert(value);
        }
        cx.notify();
    }

    fn apply_local_filter(&mut self, cx: &mut Context<Self>) {
        let Some(popover) = self.local_filter_popover.clone() else {
            return;
        };
        let mut values = self.local_filter_draft_values.clone();
        let manual_value = self.local_filter_value.trim();
        if !manual_value.is_empty() {
            values.insert(manual_value.to_string());
        }
        if values.is_empty() {
            self.clear_local_filter_for(popover.tab_id, popover.field_name.as_str(), cx);
            return;
        }
        self.local_table_filters
            .entry(popover.tab_id)
            .or_default()
            .insert(popover.field_name.clone(), values);
        self.local_filter_popover = None;
        self.refresh_active_data_table(popover.tab_id, cx);
        self.sync_table_hover_overlay_block(cx);
    }

    fn clear_local_filter(&mut self, cx: &mut Context<Self>) {
        let Some(popover) = self.local_filter_popover.clone() else {
            return;
        };
        self.clear_local_filter_for(popover.tab_id, popover.field_name.as_str(), cx);
    }

    fn clear_local_filter_for(&mut self, tab_id: TabId, field_name: &str, cx: &mut Context<Self>) {
        if let Some(filters) = self.local_table_filters.get_mut(&tab_id) {
            filters.remove(field_name);
            if filters.is_empty() {
                self.local_table_filters.remove(&tab_id);
            }
        }
        self.local_filter_popover = None;
        self.refresh_active_data_table(tab_id, cx);
        self.sync_table_hover_overlay_block(cx);
    }

    fn cancel_local_filter(&mut self, cx: &mut Context<Self>) {
        self.local_filter_popover = None;
        self.sync_table_hover_overlay_block(cx);
        cx.notify();
    }

}
