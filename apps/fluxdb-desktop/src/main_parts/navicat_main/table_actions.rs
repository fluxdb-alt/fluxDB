impl NavicatMain {
    fn refresh_table_object(&mut self, object_path: &ObjectPath, cx: &mut Context<Self>) {
        let tab_refreshes = self
            .controller
            .state()
            .tabs
            .iter()
            .filter_map(|tab| match &tab.kind {
                TabKind::DataEditor(editor) if editor.object == *object_path => Some((
                    tab.id,
                    editor.table_info.open.then_some(editor.table_info.active_tab),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        for (tab_id, info_tab) in tab_refreshes {
            self.request_data_editor_refresh(tab_id, cx);
            if let Some(info_tab) = info_tab {
                self.dispatch(AppCommand::SelectTableInfoTab { tab_id, tab: info_tab }, cx);
            }
        }

        self.show_message("已请求刷新表", AppMessageKind::Success, cx);
    }

    fn open_rename_table_modal(
        &mut self,
        object_path: ObjectPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_rename_table = Some(PendingRenameTable {
            new_name: object_path.name.clone(),
            object_path,
            error: None,
        });
        let name = self
            .pending_rename_table
            .as_ref()
            .map(|form| form.new_name.clone())
            .unwrap_or_default();
        self.rename_table_input.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn cancel_rename_table_modal(&mut self, cx: &mut Context<Self>) {
        if self._rename_table_task.is_some() {
            return;
        }
        self.pending_rename_table = None;
        cx.notify();
    }

    fn open_copy_table_modal(
        &mut self,
        object_path: ObjectPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_name = format!("{}_copy", object_path.name);
        let needs_ddl = self.table_database_kind(&object_path) == Some(DatabaseKind::Sqlite);
        self.pending_copy_table = Some(PendingCopyTable {
            object_path: object_path.clone(),
            new_name: new_name.clone(),
            copy_data: false,
            source_ddl: (!needs_ddl).then(|| Ok(String::new())),
            error: None,
        });
        self.copy_table_input.update(cx, |input, cx| {
            input.set_value(new_name, window, cx);
            input.focus(window, cx);
        });
        if needs_ddl {
            self.load_copy_table_source_ddl(object_path, cx);
        }
        cx.notify();
    }

    fn cancel_copy_table_modal(&mut self, cx: &mut Context<Self>) {
        if self._copy_table_task.is_some() {
            return;
        }
        self.pending_copy_table = None;
        cx.notify();
    }

    fn open_danger_table_modal(
        &mut self,
        object_path: ObjectPath,
        action: DangerTableAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_danger_table_action = Some(PendingDangerTableAction {
            object_path,
            action,
            foreign_key_check: ForeignKeyCheckMode::Default,
            acknowledged: false,
            error: None,
        });
        self.danger_table_foreign_key_check_select
            .update(cx, |select, cx| {
                select.set_selected_index(
                    danger_table_foreign_key_check_index(ForeignKeyCheckMode::Default),
                    window,
                    cx,
                );
            });
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn cancel_danger_table_modal(&mut self, cx: &mut Context<Self>) {
        if self._danger_table_task.is_some() {
            return;
        }
        self.pending_danger_table_action = None;
        cx.notify();
    }

    fn set_danger_table_acknowledged(&mut self, acknowledged: bool, cx: &mut Context<Self>) {
        if self._danger_table_task.is_some() {
            return;
        }
        if let Some(form) = &mut self.pending_danger_table_action {
            form.acknowledged = acknowledged;
            form.error = None;
            cx.notify();
        }
    }

    fn set_copy_table_data_mode(&mut self, copy_data: bool, cx: &mut Context<Self>) {
        if self._copy_table_task.is_some() {
            return;
        }
        if let Some(form) = &mut self.pending_copy_table {
            form.copy_data = copy_data;
            form.error = None;
            cx.notify();
        }
    }

    fn load_copy_table_source_ddl(&mut self, object_path: ObjectPath, cx: &mut Context<Self>) {
        let controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let object_for_load = object_path.clone();
            let result = cx
                .background_spawn(async move { controller.load_table_ddl(&object_for_load) })
                .await
                .map_err(|error| error.message);

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._copy_table_ddl_task = None;
                    if let Some(form) = &mut this.pending_copy_table
                        && form.object_path == object_path
                    {
                        form.source_ddl = Some(result);
                        cx.notify();
                    }
                });
            });
        });
        self._copy_table_ddl_task = Some(task);
    }

    fn copy_table_structure(&mut self, object_path: ObjectPath, cx: &mut Context<Self>) {
        if self._copy_table_structure_task.is_some() {
            self.show_message("正在复制表结构", AppMessageKind::Warning, cx);
            return;
        }

        self.show_message("正在复制表结构", AppMessageKind::Success, cx);
        let controller = self.controller.clone();
        let table_name = object_path.name.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move { controller.load_table_ddl(&object_path) })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._copy_table_structure_task = None;
                    match result {
                        Ok(ddl) => {
                            cx.write_to_clipboard(ClipboardItem::new_string(ddl));
                            this.show_message(
                                format!("已复制 {table_name} 表结构"),
                                AppMessageKind::Success,
                                cx,
                            );
                        }
                        Err(error) => {
                            this.show_message(error.message, AppMessageKind::Error, cx);
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._copy_table_structure_task = Some(task);
        cx.notify();
    }

    fn rename_table_sql_for_form(&self, form: &PendingRenameTable) -> Result<String, String> {
        let Some(database_kind) = self.table_database_kind(&form.object_path) else {
            return Err("连接不存在".to_string());
        };
        rename_table_sql_preview(database_kind, &form.object_path.name, &form.new_name)
    }

    fn copy_table_sql_for_form(&self, form: &PendingCopyTable) -> Result<String, String> {
        let Some(database_kind) = self.table_database_kind(&form.object_path) else {
            return Err("连接不存在".to_string());
        };
        let source_ddl = if database_kind == DatabaseKind::Sqlite {
            match &form.source_ddl {
                Some(Ok(ddl)) => Some(ddl.as_str()),
                Some(Err(message)) => return Err(message.clone()),
                None => return Err("正在读取原表结构".to_string()),
            }
        } else {
            None
        };
        copy_table_sql_preview_with_source_ddl(
            database_kind,
            &form.object_path.name,
            &form.new_name,
            form.copy_data,
            source_ddl,
        )
    }

    fn danger_table_sql_for_form(&self, form: &PendingDangerTableAction) -> Result<String, String> {
        let Some(database_kind) = self.table_database_kind(&form.object_path) else {
            return Err("连接不存在".to_string());
        };
        match form.action {
            DangerTableAction::Drop => drop_table_sql_preview(
                database_kind,
                &form.object_path.name,
                form.foreign_key_check,
            ),
            DangerTableAction::Truncate => truncate_table_sql_preview(
                database_kind,
                &form.object_path.name,
                form.foreign_key_check,
            ),
        }
    }

    fn confirm_rename_table(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut form) = self.pending_rename_table.clone() else {
            return;
        };
        if self._rename_table_task.is_some() {
            self.show_message("正在重命名表", AppMessageKind::Warning, cx);
            return;
        }
        let new_name = form.new_name.trim().to_string();
        if form.object_path.name.trim() == new_name {
            cx.notify();
            return;
        }
        if let Err(message) = self.rename_table_sql_for_form(&form) {
            form.error = Some(message.clone());
            self.pending_rename_table = Some(form);
            self.show_message(message, AppMessageKind::Warning, cx);
            cx.notify();
            return;
        }

        self.show_message("正在重命名表", AppMessageKind::Success, cx);
        let mut controller = self.controller.clone();
        let object = form.object_path.clone();
        let task = cx.spawn(async move |view, cx| {
            let (controller, event) = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::RenameTable {
                        object: object.clone(),
                        new_name: new_name.clone(),
                    });
                    (controller, event)
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._rename_table_task = None;
                    match event {
                        AppEvent::TableRenamed { object, new_name } => {
                            let mut renamed = object.clone();
                            renamed.name = new_name.clone();
                            this.controller
                                .merge_renamed_table_from(&controller, &object, &new_name);
                            this.pending_rename_table = None;
                            if let Some(parent) = database_parent_path(&object) {
                                let database = parent
                                    .database
                                    .clone()
                                    .unwrap_or_else(|| parent.name.clone());
                                this.load_database_children(
                                    parent,
                                    database_tree_key(object.connection_id, &database),
                                    cx,
                                );
                            }
                            let tab_ids = this
                                .controller
                                .state()
                                .tabs
                                .iter()
                                .filter_map(|tab| match &tab.kind {
                                    TabKind::DataEditor(editor)
                                        if editor.object == renamed && editor.loading =>
                                    {
                                        Some(tab.id)
                                    }
                                    _ => None,
                                })
                                .collect::<Vec<_>>();
                            for tab_id in tab_ids {
                                this.start_data_page_load_if_needed(tab_id, cx);
                            }
                            this.show_message("表已重命名", AppMessageKind::Success, cx);
                        }
                        AppEvent::Failed(error) => {
                            this.controller.merge_last_error_from(&controller);
                            if let Some(form) = &mut this.pending_rename_table {
                                form.error = Some(error.message.clone());
                            }
                            this.show_message(error.message, AppMessageKind::Error, cx);
                        }
                        _ => {
                            this.show_message("重命名表没有返回结果", AppMessageKind::Error, cx);
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._rename_table_task = Some(task);
        cx.notify();
    }

    fn confirm_copy_table(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut form) = self.pending_copy_table.clone() else {
            return;
        };
        if self._copy_table_task.is_some() {
            self.show_message("正在复制表", AppMessageKind::Warning, cx);
            return;
        }
        let new_name = form.new_name.trim().to_string();
        if let Err(message) = self.copy_table_sql_for_form(&form) {
            form.error = Some(message.clone());
            self.pending_copy_table = Some(form);
            self.show_message(message, AppMessageKind::Warning, cx);
            cx.notify();
            return;
        }

        self.show_message("正在复制表", AppMessageKind::Success, cx);
        let mut controller = self.controller.clone();
        let object = form.object_path.clone();
        let copy_data = form.copy_data;
        let task = cx.spawn(async move |view, cx| {
            let (controller, event) = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::CopyTable {
                        object: object.clone(),
                        new_name,
                        copy_data,
                    });
                    (controller, event)
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._copy_table_task = None;
                    match event {
                        AppEvent::TableCopied { object, new_name } => {
                            this.pending_copy_table = None;
                            if let Some(parent) = database_parent_path(&object) {
                                let database = parent
                                    .database
                                    .clone()
                                    .unwrap_or_else(|| parent.name.clone());
                                this.load_database_children(
                                    parent,
                                    database_tree_key(object.connection_id, &database),
                                    cx,
                                );
                            }
                            this.show_message(
                                format!("表已复制为 {new_name}"),
                                AppMessageKind::Success,
                                cx,
                            );
                        }
                        AppEvent::Failed(error) => {
                            this.controller.merge_last_error_from(&controller);
                            if let Some(form) = &mut this.pending_copy_table {
                                form.error = Some(error.message.clone());
                            }
                            this.show_message(error.message, AppMessageKind::Error, cx);
                        }
                        _ => {
                            this.show_message("复制表没有返回结果", AppMessageKind::Error, cx);
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._copy_table_task = Some(task);
        cx.notify();
    }

    fn confirm_danger_table_action(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut form) = self.pending_danger_table_action.clone() else {
            return;
        };
        if self._danger_table_task.is_some() {
            self.show_message(form.action.running_message(), AppMessageKind::Warning, cx);
            return;
        }
        if !form.acknowledged {
            form.error = Some("请先勾选确认".to_string());
            self.pending_danger_table_action = Some(form);
            cx.notify();
            return;
        }
        if let Err(message) = self.danger_table_sql_for_form(&form) {
            form.error = Some(message.clone());
            self.pending_danger_table_action = Some(form);
            self.show_message(message, AppMessageKind::Warning, cx);
            cx.notify();
            return;
        }

        self.show_message(form.action.running_message(), AppMessageKind::Success, cx);
        let mut controller = self.controller.clone();
        let object = form.object_path.clone();
        let action = form.action;
        let task = cx.spawn(async move |view, cx| {
            let (controller, event) = cx
                .background_spawn(async move {
                    let command = match action {
                        DangerTableAction::Drop => AppCommand::DropTable {
                            object: object.clone(),
                            foreign_key_check: form.foreign_key_check,
                        },
                        DangerTableAction::Truncate => AppCommand::TruncateTable {
                            object: object.clone(),
                            foreign_key_check: form.foreign_key_check,
                        },
                    };
                    let event = controller.dispatch(command);
                    (controller, event)
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._danger_table_task = None;
                    match event {
                        AppEvent::TableDropped(object) => {
                            let previous_tabs = this
                                .controller
                                .state()
                                .tabs
                                .iter()
                                .map(|tab| tab.id)
                                .collect::<Vec<_>>();
                            this.controller.merge_dropped_table_from(&controller, &object);
                            this.apply_closed_tabs(previous_tabs, cx);
                            this.pending_danger_table_action = None;
                            this.refresh_database_children_for_table(&object, cx);
                            this.show_message("表已删除", AppMessageKind::Success, cx);
                        }
                        AppEvent::TableTruncated(object) => {
                            this.pending_danger_table_action = None;
                            this.refresh_open_data_tabs_for_table(&object, cx);
                            this.refresh_database_children_for_table(&object, cx);
                            this.show_message("表已清空", AppMessageKind::Success, cx);
                        }
                        AppEvent::Failed(error) => {
                            this.controller.merge_last_error_from(&controller);
                            if let Some(form) = &mut this.pending_danger_table_action {
                                form.error = Some(error.message.clone());
                            }
                            this.show_message(error.message, AppMessageKind::Error, cx);
                        }
                        _ => {
                            this.show_message("表操作没有返回结果", AppMessageKind::Error, cx);
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._danger_table_task = Some(task);
        cx.notify();
    }

    fn refresh_database_children_for_table(&mut self, object: &ObjectPath, cx: &mut Context<Self>) {
        if let Some(parent) = database_parent_path(object) {
            let database = parent
                .database
                .clone()
                .unwrap_or_else(|| parent.name.clone());
            self.load_database_children(parent, database_tree_key(object.connection_id, &database), cx);
        }
    }

    fn refresh_open_data_tabs_for_table(&mut self, object: &ObjectPath, cx: &mut Context<Self>) {
        let tab_refreshes = self
            .controller
            .state()
            .tabs
            .iter()
            .filter_map(|tab| match &tab.kind {
                TabKind::DataEditor(editor) if editor.object == *object => Some((
                    tab.id,
                    editor.table_info.open.then_some(editor.table_info.active_tab),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        for (tab_id, info_tab) in tab_refreshes {
            self.request_data_editor_refresh(tab_id, cx);
            if let Some(info_tab) = info_tab {
                self.dispatch(AppCommand::SelectTableInfoTab { tab_id, tab: info_tab }, cx);
            }
        }
    }

    fn table_database_kind(&self, object_path: &ObjectPath) -> Option<DatabaseKind> {
        self.controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == object_path.connection_id)
            .map(|connection| connection.config.kind)
    }
}

fn database_parent_path(object_path: &ObjectPath) -> Option<ObjectPath> {
    let database = object_path.database.clone()?;
    Some(ObjectPath {
        connection_id: object_path.connection_id,
        database: Some(database.clone()),
        schema: None,
        name: database,
        kind: ObjectKind::Database,
    })
}
