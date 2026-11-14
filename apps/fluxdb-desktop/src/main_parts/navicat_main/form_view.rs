// 数据页表单视图：只改布局，编辑仍走既有的 `EditDataCell` 改动集。
//
// 约束：
// - 表单不新增一套保存语义，保存/还原复用底部工具栏的提交与丢弃命令。
// - 输入框按列复用；换行时才回填，避免每帧覆盖正在输入的内容。
impl NavicatMain {
    fn data_view_mode(&self, tab_id: TabId) -> DataViewMode {
        self.data_view_modes
            .get(&tab_id)
            .copied()
            .unwrap_or_default()
    }

    fn set_data_view_mode(&mut self, tab_id: TabId, mode: DataViewMode, cx: &mut Context<Self>) {
        if self.data_view_mode(tab_id) == mode {
            return;
        }
        self.data_view_modes.insert(tab_id, mode);
        // 表单提交/还原不重建表格委托，切回表格时统一刷新成控制器里的最新页面与改动。
        if mode == DataViewMode::Grid {
            self.refresh_active_data_table(tab_id, cx);
        }
        // 切到表单且尚未选行时落到第一行，避免空白表单；没有数据时保持空态。
        if mode == DataViewMode::Form
            && self.data_form_display_row(tab_id, cx).is_none()
            && let Some(table_state) = self.data_table_states.get(&tab_id).cloned()
        {
            table_state.update(cx, |table, cx| {
                if !table.delegate().page.rows.is_empty() {
                    table.delegate_mut().select_row_for_click(0, false);
                    cx.notify();
                }
            });
            self.reset_data_form_row_sync(tab_id);
        }
        cx.notify();
    }

    /// 当前表单行的源行下标（表格委托的显示行 → 源页行）。
    fn data_form_source_row(&self, tab_id: TabId, cx: &App) -> Option<usize> {
        let table_state = self.data_table_states.get(&tab_id)?;
        let table = table_state.read(cx);
        let delegate = table.delegate();
        let display_row = delegate
            .selected_row
            .or_else(|| delegate.selected_cell.map(|(row, _)| row))?;
        Some(
            delegate
                .source_row_indexes
                .get(display_row)
                .copied()
                .unwrap_or(display_row),
        )
    }

    /// 表单行数（显示页），用于「上一条 / 下一条」边界判断。
    fn data_form_display_rows(&self, tab_id: TabId, cx: &App) -> usize {
        self.data_table_states
            .get(&tab_id)
            .map(|state| state.read(cx).delegate().page.rows.len())
            .unwrap_or(0)
    }

    fn data_form_display_row(&self, tab_id: TabId, cx: &App) -> Option<usize> {
        let table_state = self.data_table_states.get(&tab_id)?;
        let table = table_state.read(cx);
        let delegate = table.delegate();
        delegate
            .selected_row
            .or_else(|| delegate.selected_cell.map(|(row, _)| row))
    }

    /// 按列确保输入框存在；首次创建时订阅提交事件。
    ///
    /// 回车或失焦时提交当前字段；切行/保存前还会由 `commit_dirty_form_inputs`
    /// 统一提交一遍。切行时输入框会被强制回填成新行值，失焦不会把上一行文本写到新行。
    fn ensure_data_form_inputs(
        &mut self,
        tab_id: TabId,
        columns: &[GdbColumn],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (index, column) in columns.iter().enumerate() {
            let key = (tab_id, index);
            if self.data_form_inputs.contains_key(&key) {
                continue;
            }
            // 占位符只表达 NULL；类型已在标签下方展示，重复类型会让表单显得嘈杂。
            let placeholder = if column.nullable {
                tr("app.data.form.null_placeholder")
            } else {
                SharedString::default()
            };
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            let subscription = cx.subscribe(
                &input,
                move |this: &mut Self, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.commit_form_input(tab_id, index, input.read(cx).value().to_string(), cx);
                        cx.stop_propagation();
                    } else if matches!(event, InputEvent::Blur) {
                        // 失焦即提交：否则离开输入框后每帧同步会用页面旧值把刚输入的内容清掉。
                        this.commit_form_input(tab_id, index, input.read(cx).value().to_string(), cx);
                        // 内容未变时 commit 不会通知，单独重绘一次以撤掉焦点描边。
                        cx.notify();
                    } else if matches!(event, InputEvent::Focus) {
                        // 焦点描边画在字段外壳上，需要父视图重绘一次（点进/点出各一次）。
                        cx.notify();
                    } else if matches!(event, InputEvent::Change)
                        && !this.data_form_syncing_inputs
                    {
                        // 只刷新底栏实体（保存/还原按钮的可用状态），不惊动整个窗口。
                        // 程序回填输入框时置位标记，避免切行时 N 个输入框各刷新一次底栏。
                        if let Some(footer) = this.data_form_footers.get(&tab_id) {
                            footer.update(cx, |_, cx| cx.notify());
                        }
                    }
                },
            );
            self.data_form_inputs.insert(key, input);
            self._data_form_input_subscriptions.insert(key, subscription);
        }
    }

    /// 把页面值回填到输入框。
    ///
    /// 同一行每帧也会同步（页面重载、还原后值可能变化），但跳过正在输入的框；
    /// 未被强制同步、且输入框文本已经偏离上次回填值时，视为用户未提交的编辑，
    /// 保留输入框内容——否则失焦提交失败（或事件顺序靠后）时会被页面旧值清空。
    /// 换行/还原时 `force` 强制覆盖，避免上一行未提交的文本留在新行。
    fn sync_data_form_inputs(
        &mut self,
        tab_id: TabId,
        values: &[CellValue],
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 回填期间屏蔽 Change：切行时 N 个输入框不必各自刷新一次底栏。
        self.data_form_syncing_inputs = true;
        for (index, value) in values.iter().enumerate() {
            let key = (tab_id, index);
            let Some(input) = self.data_form_inputs.get(&key).cloned() else {
                continue;
            };
            if !force && input.read(cx).focus_handle(cx).is_focused(window) {
                continue;
            }
            let current = input.read(cx).value().to_string();
            // 已与页面值一致：记下当前文本即可，不必为大字段生成一份完整副本。
            if data_cell_edit_text_unchanged(value, current.as_str()) {
                self.data_form_synced_texts.insert(key, current);
                continue;
            }
            let edited_by_user = !force
                && self
                    .data_form_synced_texts
                    .get(&key)
                    .is_some_and(|synced| synced != &current);
            if edited_by_user {
                continue;
            }
            let text = data_cell_edit_text(value);
            input.update(cx, |input, cx| {
                input.set_value(text.clone(), window, cx);
            });
            self.data_form_synced_texts.insert(key, text);
        }
        self.data_form_syncing_inputs = false;
    }

    fn reset_data_form_row_sync(&mut self, tab_id: TabId) {
        self.data_form_synced_row.remove(&tab_id);
        // 换行/还原后旧行的候选菜单不再对应当前记录，直接收起。
        self.data_form_choice_menus
            .retain(|(menu_tab, _)| *menu_tab != tab_id);
    }

    /// 展开某个字段的候选下拉（同一标签页同时只开一个）。
    ///
    /// 外部点击由菜单自己的 `on_mouse_down_out` 关闭；箭头只负责打开，
    /// 避免“外部点击关闭 + 箭头切换”在一次事件里互相打架。
    fn open_data_form_choice_menu(&mut self, tab_id: TabId, column: usize, cx: &mut Context<Self>) {
        self.data_form_choice_menus
            .retain(|(menu_tab, _)| *menu_tab != tab_id);
        self.data_form_choice_menus.insert((tab_id, column));
        cx.notify();
    }

    fn close_data_form_choice_menu(&mut self, tab_id: TabId, column: usize, cx: &mut Context<Self>) {
        if self.data_form_choice_menus.remove(&(tab_id, column)) {
            cx.notify();
        }
    }

    /// 表单字段回车提交：只写当前行当前列，走与表格单元格相同的改动集。
    fn commit_form_input(
        &mut self,
        tab_id: TabId,
        column: usize,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let Some(source_row) = self.data_form_source_row(tab_id, cx) else {
            return;
        };
        let Some(column_meta) = self.data_form_column_meta(tab_id, source_row, column) else {
            return;
        };
        let Some(current) = self.data_form_cell_value(tab_id, source_row, column) else {
            return;
        };
        if data_cell_edit_text_unchanged(&current, text.as_str()) {
            return;
        }
        let value = if text.is_empty() && column_meta.nullable {
            CellValue::Null
        } else {
            match data_cell_value_from_text(&column_meta.meta, text.as_str()) {
                Ok(value) => value,
                Err(_) => {
                    self.show_message(
                        tr("app.data.error.type_mismatch").as_ref(),
                        AppMessageKind::Warning,
                        cx,
                    );
                    self.reset_data_form_row_sync(tab_id);
                    cx.notify();
                    return;
                }
            }
        };
        self.dispatch(
            AppCommand::EditDataCell {
                tab_id,
                row: source_row,
                column,
                value,
            },
            cx,
        );
        // 表单提交不重建表格委托：当前看不到表格，逐次重建纯属开销；
        // 切回表格视图时 `set_data_view_mode` 会统一刷新。
        // 清掉同步标记，下一次非聚焦同步会把输入框改写成规范化后的值（如 `01` → `1`）。
        self.data_form_synced_texts.remove(&(tab_id, column));
        cx.notify();
    }

    /// 保存/切行前提交所有与页面值不同的表单字段。
    fn commit_dirty_form_inputs(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(source_row) = self.data_form_source_row(tab_id, cx) else {
            return;
        };
        let Some(page) = self.data_form_page(tab_id) else {
            return;
        };
        let Some(row) = page.rows.get(source_row) else {
            return;
        };
        // 先取出所有偏离页面值的字段，避免在提交时继续借用页面。
        let dirty = self
            .data_form_inputs
            .iter()
            .filter(|((input_tab, _), _)| *input_tab == tab_id)
            .filter_map(|((_, index), input)| {
                let current = row.values.get(*index)?;
                let text = input.read(cx).value().to_string();
                (!data_cell_edit_text_unchanged(current, text.as_str())).then_some((*index, text))
            })
            .collect::<Vec<_>>();
        for (index, text) in dirty {
            self.commit_form_input(tab_id, index, text, cx);
        }
    }

    /// 「上一条 / 下一条」：先落盘当前表单编辑，再移动选中行。
    fn navigate_data_form_row(&mut self, tab_id: TabId, delta: isize, cx: &mut Context<Self>) {
        self.commit_dirty_form_inputs(tab_id, cx);
        let Some(table_state) = self.data_table_states.get(&tab_id).cloned() else {
            return;
        };
        let current = self.data_form_display_row(tab_id, cx).unwrap_or(0);
        let rows = self.data_form_display_rows(tab_id, cx);
        if rows == 0 {
            return;
        }
        let next = (current as isize + delta).clamp(0, rows as isize - 1) as usize;
        table_state.update(cx, |table, cx| {
            let delegate = table.delegate_mut();
            delegate.select_row_for_click(next, false);
            cx.notify();
        });
        // 行变化后重新回填输入框。
        self.reset_data_form_row_sync(tab_id);
        cx.notify();
    }

    /// 表单保存：先提交表单字段，再走既有的改动提交确认流程。
    fn save_data_form(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.commit_dirty_form_inputs(tab_id, cx);
        let has_changes = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => editor.changes.as_ref(),
                _ => None,
            })
            .is_some_and(|changes| !changes.is_empty());
        if has_changes {
            self.request_apply_data_changes(tab_id, cx);
        }
    }

    /// 表单还原：丢弃改动集并把输入框恢复成页面值。
    ///
    /// 同样不重建表格委托，切回表格视图时再刷新。
    fn revert_data_form(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.dispatch(AppCommand::DiscardDataChanges(tab_id), cx);
        self.data_change_sql_preview_tabs.remove(&tab_id);
        if self.pending_apply_data_changes == Some(tab_id) {
            self.pending_apply_data_changes = None;
        }
        self.reset_data_form_row_sync(tab_id);
        cx.notify();
    }

    fn data_form_page(&self, tab_id: TabId) -> Option<&DataPage> {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => editor.page.as_ref(),
                _ => None,
            })
    }

    fn data_form_cell_value(
        &self,
        tab_id: TabId,
        row: usize,
        column: usize,
    ) -> Option<CellValue> {
        self.data_form_page(tab_id)
            .and_then(|page| page.rows.get(row))
            .and_then(|row| row.values.get(column))
            .cloned()
    }

    fn data_form_column_meta(
        &self,
        tab_id: TabId,
        _row: usize,
        column: usize,
    ) -> Option<FormColumnMeta> {
        let page = self.data_form_page(tab_id)?;
        let choices = self
            .column_choices_for_tab(tab_id)
            .get(&page.columns.get(column)?.name)
            .cloned()
            .unwrap_or_default();
        page.columns.get(column).map(|column| FormColumnMeta {
            nullable: column.nullable,
            meta: form_data_column_meta(column, choices),
        })
    }

    /// 单列可跳转目标：物理外键优先，其后是 ER 设计关系；复合关系留给关系面板。
    fn form_foreign_key_targets(
        &self,
        tab_id: TabId,
        object: &ObjectPath,
    ) -> Vec<ForeignKeyInfo> {
        let mut targets = self
            .foreign_keys_for_tab(tab_id)
            .map(|(_, foreign_keys)| foreign_keys.to_vec())
            .unwrap_or_default();
        if let Some(relationships) = self.data_logical_relationships.get(&tab_id) {
            for foreign_key in logical_relationship_foreign_keys(object, relationships) {
                if targets
                    .iter()
                    .any(|existing| existing.column == foreign_key.column)
                {
                    continue;
                }
                targets.push(foreign_key);
            }
        }
        targets
    }
}

/// 表单字段所需的最小列信息（类型、可空、主键与转换用 meta）。
struct FormColumnMeta {
    nullable: bool,
    meta: DataTableColumnMeta,
}

/// 表单底栏独立实体。
///
/// 保存/还原按钮的可用状态依赖输入框文本，但输入框的 `Change` 只通知本实体，
/// 不再让整个 `NavicatMain` 逐键重建。
struct DataFormFooter {
    parent: WeakEntity<NavicatMain>,
    tab_id: TabId,
    inputs: Vec<(usize, Entity<InputState>)>,
    colors: UiColors,
}

impl NavicatMain {
    /// 取（或首次创建）某标签页的表单底栏，并把最新输入框、配色同步进去。
    fn ensure_data_form_footer(
        &mut self,
        tab_id: TabId,
        colors: UiColors,
        cx: &mut Context<Self>,
    ) -> Entity<DataFormFooter> {
        let inputs = self
            .data_form_inputs
            .iter()
            .filter(|((input_tab, _), _)| *input_tab == tab_id)
            .map(|((_, index), input)| (*index, input.clone()))
            .collect::<Vec<_>>();
        if let Some(footer) = self.data_form_footers.get(&tab_id).cloned() {
            footer.update(cx, |footer, cx| {
                footer.inputs = inputs;
                footer.colors = colors;
                cx.notify();
            });
            return footer;
        }
        let parent = cx.entity().downgrade();
        let footer = cx.new(|_| DataFormFooter {
            parent,
            tab_id,
            inputs,
            colors,
        });
        self.data_form_footers.insert(tab_id, footer.clone());
        footer
    }
}

impl Render for DataFormFooter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let Some(parent) = self.parent.upgrade() else {
            return div();
        };
        // 待保存内容 = 已进入改动集，或任一输入框文本与页面值不同（尚未回车/失焦的编辑也算）。
        let has_pending = {
            let main = parent.read(cx);
            let values = main
                .data_form_source_row(self.tab_id, cx)
                .and_then(|row| {
                    main.data_form_page(self.tab_id)
                        .and_then(|page| page.rows.get(row))
                })
                .map(|row| row.values.as_slice());
            main.data_editor_has_dirty_changes(self.tab_id)
                || values.is_some_and(|values| {
                    self.inputs.iter().any(|(index, input)| {
                        values.get(*index).is_some_and(|value| {
                            !data_cell_edit_text_unchanged(value, input.read(cx).value().as_ref())
                        })
                    })
                })
        };
        data_form_foot(self.tab_id, has_pending, colors, cx)
    }
}
