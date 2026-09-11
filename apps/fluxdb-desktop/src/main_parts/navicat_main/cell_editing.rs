impl NavicatMain {
    fn begin_data_cell_edit(
        &mut self,
        state: DataCellEditState,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.data_cell_editing = Some(state);
        self.temporal_part_editing = None;
        self.data_cell_edit_input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn commit_data_cell_edit(&mut self, cx: &mut Context<Self>) {
        let _ = self.commit_data_cell_edit_with_refresh(true, cx);
    }

    fn commit_data_cell_edit_with_refresh(
        &mut self,
        refresh_table: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(editing) = self.data_cell_editing.take() else {
            return true;
        };
        let value = self.data_cell_edit_input.read(cx).value().to_string();
        if self
            .current_data_cell_value(editing)
            .as_ref()
            .is_some_and(|current| data_cell_edit_text_unchanged(current, value.as_str()))
        {
            if refresh_table {
                self.refresh_active_data_table(editing.tab_id, cx);
            }
            cx.notify();
            return true;
        }
        let Some(meta) = self.data_cell_meta_for_edit(editing) else {
            self.show_message("找不到当前字段，无法保存编辑", AppMessageKind::Error, cx);
            self.data_cell_editing = Some(editing);
            cx.notify();
            return false;
        };
        let Ok(value) = data_cell_value_from_text(&meta, value.as_str()) else {
            self.show_message("输入值不符合字段类型", AppMessageKind::Warning, cx);
            self.data_cell_editing = Some(editing);
            cx.notify();
            return false;
        };
        self.apply_data_cell_edit_value(editing, value, refresh_table, cx);
        true
    }

    fn current_data_cell_value(&self, editing: DataCellEditState) -> Option<CellValue> {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == editing.tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => editor.page.as_ref(),
                TabKind::QueryEditor(editor) => {
                    active_query_result_editor_state(editor).and_then(|editor| editor.page.as_ref())
                }
                _ => None,
            })
            .and_then(|page| page.rows.get(editing.source_row))
            .and_then(|row| row.values.get(editing.source_col))
            .cloned()
    }

    fn data_cell_meta_for_edit(&self, editing: DataCellEditState) -> Option<DataTableColumnMeta> {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == editing.tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => editor.page.as_ref(),
                TabKind::QueryEditor(editor) => {
                    active_query_result_editor_state(editor).and_then(|editor| editor.page.as_ref())
                }
                _ => None,
            })
            .and_then(|page| page.columns.get(editing.source_col).cloned())
            .map(|column| DataTableColumnMeta {
                choices: self.column_choices_for_editing_cell(editing, &column.name),
                name: column.name,
                type_name: column.type_name.unwrap_or_else(|| "unknown".to_string()),
                comment: column.comment,
                nullable: column.nullable,
                primary_key: column.primary_key,
            })
    }

    fn commit_data_cell_edit_before_cell_change(
        &mut self,
        target: DataCellEditState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !data_cell_edit_should_commit_before_cell_change(self.data_cell_editing, target) {
            return true;
        }

        let Some(editing) = self.data_cell_editing else {
            return true;
        };
        let tab_id = editing.tab_id;
        if !self.commit_data_cell_edit_with_refresh(false, cx) {
            return false;
        }

        cx.defer_in(window, move |this, _, cx| {
            this.refresh_active_data_table(tab_id, cx);
        });
        true
    }

    fn apply_data_cell_edit_value(
        &mut self,
        editing: DataCellEditState,
        value: CellValue,
        refresh_table: bool,
        cx: &mut Context<Self>,
    ) {
        self.data_cell_editing = None;
        self.temporal_part_editing = None;
        self.dispatch(
            AppCommand::EditDataCell {
                tab_id: editing.tab_id,
                row: editing.source_row,
                column: editing.source_col,
                value,
            },
            cx,
        );
        if refresh_table {
            self.refresh_active_data_table(editing.tab_id, cx);
        }
        cx.notify();
    }

    fn cancel_data_cell_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(editing) = self.data_cell_editing.take() else {
            return false;
        };
        self.temporal_part_editing = None;
        self.refresh_active_data_table(editing.tab_id, cx);
        cx.notify();
        true
    }

    fn begin_temporal_part_edit(
        &mut self,
        state: TemporalPartEditState,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.temporal_part_editing = Some(state);
        self.temporal_part_input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn apply_temporal_part_input(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editing) = self.temporal_part_editing else {
            return;
        };
        let edit_input = match editing.target {
            TemporalEditTarget::DataCell(_) => self.data_cell_edit_input.clone(),
            TemporalEditTarget::CellDetail(_) => self.cell_detail_input.clone(),
        };
        let current = edit_input.read(cx).value().to_string();
        let Some(next) = temporal_value_after_part_input(current.as_str(), editing, value) else {
            return;
        };
        edit_input.update(cx, |input, cx| {
            input.set_value(next, window, cx);
        });
    }
}
