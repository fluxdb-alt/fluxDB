impl NavicatMain {
    /// 右键「设置快捷键…」：为当前表新建快捷组，并把表预置进组内。
    fn open_table_shortcut_dialog_for(
        &mut self,
        path: ObjectPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.controller.state().settings.table_shortcut_groups.len() >= TABLE_SHORTCUT_GROUP_LIMIT
        {
            self.show_message(
                tr("app.table_shortcuts.group_limit").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        let Some(target) = self.table_shortcut_target_for_path(&path) else {
            return;
        };
        let name_input = self.new_table_shortcut_name_input(window, cx, "");
        self.table_shortcut_dialog = Some(TableShortcutDialog {
            group_id: next_table_shortcut_group_id(),
            group_index: None,
            name_input,
            keys: String::new(),
            targets: vec![target],
            capturing: false,
            invalid: false,
            conflict: None,
            focus_handle: cx.focus_handle(),
        });
        self.start_table_shortcut_capture(window, cx);
        cx.notify();
    }

    /// 设置页「管理」：编辑已有组。
    fn open_table_shortcut_dialog_at(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = self.controller.state().settings.clone();
        let Some(group) = settings.table_shortcut_groups.get(index).cloned() else {
            return;
        };
        let name_input = self.new_table_shortcut_name_input(window, cx, &group.name);
        // 打开时先显示已有冲突；保存时会再次校验。
        let conflict = if group.keys.is_empty()
            || validate_table_shortcut_spec(&group.keys).is_err()
        {
            None
        } else {
            table_shortcut_conflict_message(&settings, &group.keys, Some(&group.id))
        };
        self.table_shortcut_dialog = Some(TableShortcutDialog {
            group_id: group.id,
            group_index: Some(index),
            name_input,
            keys: group.keys,
            targets: group.targets,
            capturing: false,
            invalid: false,
            conflict,
            focus_handle: cx.focus_handle(),
        });
        cx.notify();
    }

    /// 设置页「添加快捷组」：空组，先选表再录键。
    fn open_table_shortcut_dialog_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.controller.state().settings.table_shortcut_groups.len() >= TABLE_SHORTCUT_GROUP_LIMIT
        {
            self.show_message(
                tr("app.table_shortcuts.group_limit").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        let name_input = self.new_table_shortcut_name_input(window, cx, "");
        self.table_shortcut_dialog = Some(TableShortcutDialog {
            group_id: next_table_shortcut_group_id(),
            group_index: None,
            name_input,
            keys: String::new(),
            targets: Vec::new(),
            capturing: false,
            invalid: false,
            conflict: None,
            focus_handle: cx.focus_handle(),
        });
        self.start_table_shortcut_capture(window, cx);
        cx.notify();
    }

    fn new_table_shortcut_name_input(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        value: &str,
    ) -> Entity<InputState> {
        let value = value.to_string();
        cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(tr("app.table_shortcuts.name_placeholder"))
                .default_value(value)
        })
    }

    /// 延迟聚焦录制区：弹窗元素先挂载，下一帧才能接收按键。
    fn start_table_shortcut_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dialog) = self.table_shortcut_dialog.as_mut() {
            dialog.capturing = true;
            let focus_handle = dialog.focus_handle.clone();
            cx.defer_in(window, move |this, window, cx| {
                if this
                    .table_shortcut_dialog
                    .as_ref()
                    .is_some_and(|dialog| dialog.capturing)
                {
                    focus_handle.focus(window, cx);
                }
            });
        }
    }

    fn close_table_shortcut_dialog(&mut self, cx: &mut Context<Self>) {
        if self.table_shortcut_dialog.take().is_some() {
            self.quick_open_table_for_shortcut = false;
            self.quick_open_table_open = false;
            cx.notify();
        }
    }

    /// 录制表级快捷键：Esc 退出录制，⌫ 清除，其余按键写入并即时校验冲突。
    fn table_shortcut_dialog_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = self.controller.state().settings.clone();
        let Some(dialog) = self.table_shortcut_dialog.as_mut() else {
            return;
        };
        if !dialog.capturing {
            return;
        }
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            dialog.capturing = false;
            dialog.invalid = false;
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if matches!(key, "backspace" | "delete") {
            dialog.keys.clear();
            dialog.capturing = false;
            dialog.invalid = false;
            dialog.conflict = None;
            cx.notify();
            cx.stop_propagation();
            return;
        }
        let Some(spec) = shortcut_spec_from_keystroke(&event.keystroke) else {
            dialog.invalid = true;
            dialog.capturing = false;
            cx.notify();
            cx.stop_propagation();
            return;
        };
        dialog.invalid = validate_table_shortcut_spec(&spec).is_err();
        dialog.keys = spec.clone();
        dialog.conflict = if dialog.invalid {
            None
        } else {
            table_shortcut_conflict_message(&settings, &spec, Some(&dialog.group_id))
        };
        dialog.capturing = false;
        cx.notify();
        cx.stop_propagation();
    }

    fn remove_table_shortcut_dialog_target(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(dialog) = self.table_shortcut_dialog.as_mut()
            && index < dialog.targets.len()
        {
            dialog.targets.remove(index);
            cx.notify();
        }
    }

    fn save_table_shortcut_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.table_shortcut_dialog.clone() else {
            return;
        };
        let name = dialog.name_input.read(cx).value().trim().to_string();
        if dialog.targets.is_empty() {
            self.show_message(
                tr("app.table_shortcuts.no_tables").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        if dialog.targets.len() > TABLE_SHORTCUT_TARGET_LIMIT {
            self.show_message(
                tr("app.table_shortcuts.target_limit").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        if dialog.keys.is_empty() || validate_table_shortcut_spec(&dialog.keys).is_err() {
            self.show_message(
                tr("app.table_shortcuts.invalid_shortcut").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        let mut settings = self.controller.state().settings.clone();
        if let Some(conflict) = table_shortcut_conflict_message(
            &settings,
            &dialog.keys,
            Some(&dialog.group_id),
        ) {
            self.show_message(conflict.as_str(), AppMessageKind::Warning, cx);
            return;
        }
        let group = TableShortcutGroup {
            id: dialog.group_id.clone(),
            name,
            keys: dialog.keys.clone(),
            targets: dialog.targets.clone(),
        };
        match dialog.group_index {
            Some(index) if index < settings.table_shortcut_groups.len() => {
                settings.table_shortcut_groups[index] = group;
            }
            _ => {
                if settings.table_shortcut_groups.len() >= TABLE_SHORTCUT_GROUP_LIMIT {
                    self.show_message(
                        tr("app.table_shortcuts.group_limit").as_ref(),
                        AppMessageKind::Warning,
                        cx,
                    );
                    return;
                }
                settings.table_shortcut_groups.push(group);
            }
        }
        self.persist_table_shortcut_groups(settings, cx);
        self.table_shortcut_dialog = None;
        self.show_message(
            tr("app.table_shortcuts.saved").as_ref(),
            AppMessageKind::Success,
            cx,
        );
        cx.notify();
    }

    fn delete_table_shortcut_dialog_group(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.table_shortcut_dialog.clone() else {
            return;
        };
        let Some(index) = dialog.group_index else {
            self.close_table_shortcut_dialog(cx);
            return;
        };
        let mut settings = self.controller.state().settings.clone();
        if index >= settings.table_shortcut_groups.len() {
            return;
        }
        settings.table_shortcut_groups.remove(index);
        self.persist_table_shortcut_groups(settings, cx);
        self.table_shortcut_dialog = None;
        self.show_message(
            tr("app.table_shortcuts.deleted").as_ref(),
            AppMessageKind::Info,
            cx,
        );
        cx.notify();
    }

    /// 设置页「清除」：直接移除整组并重绑按键。
    fn clear_table_shortcut_group(&mut self, index: usize, cx: &mut Context<Self>) {
        let mut settings = self.controller.state().settings.clone();
        if index >= settings.table_shortcut_groups.len() {
            return;
        }
        settings.table_shortcut_groups.remove(index);
        self.persist_table_shortcut_groups(settings, cx);
        self.show_message(
            tr("app.table_shortcuts.deleted").as_ref(),
            AppMessageKind::Info,
            cx,
        );
    }

    /// 统一的设置落盘与按键重绑：弹窗、设置页行操作都走这里。
    fn persist_table_shortcut_groups(&mut self, settings: Settings, cx: &mut Context<Self>) {
        let before = self.controller.state().settings.clone();
        self.settings_editor_draft.table_shortcut_groups =
            settings.table_shortcut_groups.clone();
        let _ = self.controller.dispatch(AppCommand::SaveSettings(settings));
        let _ = self.storage.save_settings(&self.controller.state().settings);
        rebind_table_shortcut_groups(cx, &before, &self.controller.state().settings);
        self.sync_settings_tab_dirty();
        cx.notify();
    }

    fn table_shortcut_target_for_path(&self, path: &ObjectPath) -> Option<TableShortcutTarget> {
        let connection = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == path.connection_id)?;
        Some(TableShortcutTarget {
            connection_id: path.connection_id,
            connection_name: connection.config.name.clone(),
            database: path.database.clone(),
            schema: path.schema.clone(),
            name: path.name.clone(),
            kind: path.kind,
        })
    }

    /// 弹窗内「添加表…」：复用快速打开表弹层，覆盖所有已加载的 SQL 连接。
    fn open_quick_open_table_for_shortcut(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.table_shortcut_dialog.is_none() {
            return;
        }
        let mut entries = Vec::new();
        for connection in &self.controller.state().connections {
            if matches!(
                connection.config.kind,
                DatabaseKind::MongoDb | DatabaseKind::Redis
            ) {
                continue;
            }
            entries.extend(
                connection
                    .objects
                    .iter()
                    .filter(|object| {
                        matches!(object.path.kind, ObjectKind::Table | ObjectKind::View)
                    })
                    .map(|object| object.path.clone()),
            );
        }
        entries.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.connection_id.cmp(&right.connection_id))
                .then_with(|| left.database.cmp(&right.database))
        });
        entries.dedup();
        if entries.is_empty() {
            self.show_message(
                tr("app.quick_open_table.not_loaded").as_ref(),
                AppMessageKind::Info,
                cx,
            );
            return;
        }
        self.quick_open_table_for_shortcut = true;
        self.quick_open_table_entries = entries;
        self.quick_open_table_search.clear();
        self.quick_open_table_selected = 0;
        self.quick_open_table_open = true;
        self.quick_open_table_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// 快速打开表选中后：加入弹窗草稿，而不是打开数据页。
    fn add_table_shortcut_target(&mut self, entry: ObjectPath, cx: &mut Context<Self>) {
        let Some(target) = self.table_shortcut_target_for_path(&entry) else {
            return;
        };
        let Some(dialog) = self.table_shortcut_dialog.as_mut() else {
            return;
        };
        if dialog.targets.len() >= TABLE_SHORTCUT_TARGET_LIMIT {
            self.show_message(
                tr("app.table_shortcuts.target_limit").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        if dialog
            .targets
            .iter()
            .any(|existing| table_shortcut_targets_match(existing, &target))
        {
            self.show_message(
                tr("app.table_shortcuts.duplicate_table").as_ref(),
                AppMessageKind::Info,
                cx,
            );
            return;
        }
        dialog.targets.push(target);
        self.quick_open_table_open = false;
        self.quick_open_table_for_shortcut = false;
        cx.notify();
    }

    fn open_table_shortcut_action(
        &mut self,
        action: &OpenTableShortcut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 编辑弹窗打开时应用快捷键可能同时触发，直接忽略，避免弹窗下打开表。
        if self.table_shortcut_dialog.is_some() {
            return;
        }
        let settings = self.controller.state().settings.clone();
        let Some(group) = settings.table_shortcut_groups.get(action.slot) else {
            return;
        };
        match group.targets.as_slice() {
            [] => {}
            [target] => self.open_table_shortcut_target(target, cx),
            _ => self.open_table_shortcut_picker(action.slot, window, cx),
        }
    }

    fn open_table_shortcut_picker(
        &mut self,
        group_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.table_shortcut_picker = Some(TableShortcutPicker {
            group_index,
            selected: 0,
            focus_handle: cx.focus_handle(),
        });
        let focus_handle = self
            .table_shortcut_picker
            .as_ref()
            .map(|picker| picker.focus_handle.clone());
        if let Some(focus_handle) = focus_handle {
            cx.defer_in(window, move |this, window, cx| {
                if this.table_shortcut_picker.is_some() {
                    focus_handle.focus(window, cx);
                }
            });
        }
        cx.notify();
    }

    fn close_table_shortcut_picker(&mut self, cx: &mut Context<Self>) {
        if self.table_shortcut_picker.take().is_some() {
            cx.notify();
        }
    }

    fn table_shortcut_picker_next(
        &mut self,
        _: &TableShortcutPickerNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = self.controller.state().settings.clone();
        let Some(picker) = self.table_shortcut_picker.as_mut() else {
            return;
        };
        let Some(group) = settings.table_shortcut_groups.get(picker.group_index) else {
            return;
        };
        let len = group.targets.len();
        if len > 0 {
            picker.selected = (picker.selected + 1) % len;
        }
        cx.notify();
    }

    fn table_shortcut_picker_previous(
        &mut self,
        _: &TableShortcutPickerPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = self.controller.state().settings.clone();
        let Some(picker) = self.table_shortcut_picker.as_mut() else {
            return;
        };
        let Some(group) = settings.table_shortcut_groups.get(picker.group_index) else {
            return;
        };
        let len = group.targets.len();
        if len > 0 {
            picker.selected = if picker.selected == 0 {
                len - 1
            } else {
                picker.selected - 1
            };
        }
        cx.notify();
    }

    fn table_shortcut_picker_confirm(
        &mut self,
        _: &TableShortcutPickerConfirm,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_table_shortcut_picker(cx);
    }

    fn table_shortcut_picker_cancel(
        &mut self,
        _: &TableShortcutPickerCancel,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_table_shortcut_picker(cx);
    }

    /// 数字直选：1–9 对应组内顺序。
    fn select_table_shortcut(
        &mut self,
        action: &SelectTableShortcut,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(picker) = self.table_shortcut_picker.as_mut() {
            picker.selected = action.slot.saturating_sub(1);
        }
        self.confirm_table_shortcut_picker(cx);
    }

    fn confirm_table_shortcut_picker(&mut self, cx: &mut Context<Self>) {
        let settings = self.controller.state().settings.clone();
        let Some(picker) = self.table_shortcut_picker.as_ref() else {
            return;
        };
        let Some(target) = settings
            .table_shortcut_groups
            .get(picker.group_index)
            .and_then(|group| group.targets.get(picker.selected))
            .cloned()
        else {
            self.close_table_shortcut_picker(cx);
            return;
        };
        self.table_shortcut_picker = None;
        self.open_table_shortcut_target(&target, cx);
    }

    /// 打开表快捷目标：连接未打开或已删除时给出明确提示，不静默连接。
    fn open_table_shortcut_target(&mut self, target: &TableShortcutTarget, cx: &mut Context<Self>) {
        let state = self.controller.state();
        let Some(connection) = state
            .connections
            .iter()
            .find(|connection| connection.config.id == target.connection_id)
        else {
            self.show_message(
                tr("app.table_shortcuts.connection_missing").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        };
        let path = ObjectPath {
            connection_id: target.connection_id,
            database: target.database.clone(),
            schema: target.schema.clone(),
            name: target.name.clone(),
            kind: target.kind,
        };
        if !connection.connected {
            // 连接未打开：登记待打开目标并复用侧边栏的连接流程，
            // 连接成功后再打开表；失败则丢弃，避免静默重试。
            self.pending_table_shortcut_open = Some((target.connection_id, path));
            self.open_connection_from_sidebar(target.connection_id, cx);
            return;
        }
        self.dispatch(AppCommand::OpenDataEditor(path), cx);
    }

    /// ⌥1–⌥9：按编号激活当前一级工作区第 N 个可见标签页。
    fn activate_tab_slot_action(
        &mut self,
        action: &ActivateTabSlot,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action.slot == 0 {
            return;
        }
        let state = self.controller.state();
        let active_scope = state.active_tab().and_then(tab_first_level_scope);
        let tabs = state
            .tabs
            .iter()
            .filter(|tab| match &active_scope {
                Some(scope) => tab_first_level_scope(tab).as_ref() == Some(scope),
                None => tab_first_level_scope(tab).is_none(),
            })
            .collect::<Vec<_>>();
        let ordered = order_tab_refs(tabs, &self.tab_order, &self.pinned_tabs);
        let Some(tab) = ordered.get(action.slot - 1) else {
            return;
        };
        let tab_id = tab.id;
        self.dispatch(AppCommand::ActivateTab(tab_id), cx);
    }
}

/// 表快捷组稳定 id：仅用于编辑时排除自身，时间戳足够且无需引入 uuid 依赖。
fn next_table_shortcut_group_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("table-group-{nanos}")
}

fn table_shortcut_targets_match(left: &TableShortcutTarget, right: &TableShortcutTarget) -> bool {
    left.connection_id == right.connection_id
        && left.database == right.database
        && left.schema == right.schema
        && left.name == right.name
        && left.kind == right.kind
}

/// 把冲突来源转成可直接展示的中文说明；无冲突返回 None。
fn table_shortcut_conflict_message(
    settings: &Settings,
    spec: &str,
    ignore_group_id: Option<&str>,
) -> Option<String> {
    match table_shortcut_conflict(settings, spec, ignore_group_id)? {
        ShortcutConflict::Reserved(label) => Some(format!(
            "{}{label}",
            tr("app.table_shortcuts.conflict_reserved")
        )),
        ShortcutConflict::AppShortcut(definition) => Some(format!(
            "{}{}",
            tr("app.table_shortcuts.conflict_app"),
            shortcut_text(definition).0
        )),
        ShortcutConflict::TableGroup(name) => Some(format!(
            "{}{name}",
            tr("app.table_shortcuts.conflict_group")
        )),
    }
}

