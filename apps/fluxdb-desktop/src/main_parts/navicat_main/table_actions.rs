impl NavicatMain {
    fn open_design_table_in_background(&mut self, object: ObjectPath, cx: &mut Context<Self>) {
        let key = table_tree_key(&object);
        if self._design_table_tasks.contains_key(&key) {
            return;
        }

        // 设计表加载用页内进度表示；重复点击同一张表仍复用现有任务。
        let finish_key = key.clone();
        let task = self.spawn_controller_command(
            AppCommand::LoadDesignTableBase(object),
            move |this, event, cx| {
                this._design_table_tasks.remove(&finish_key);
                let result = match event {
                    AppEvent::DesignTableLoaded(result) => result,
                    AppEvent::Failed(error) => Err(error),
                    _ => Err(fluxdb_core::UserFacingError {
                        title: tr("app.create_table.error.load_design_failed").to_string(),
                        message: tr("app.create_table.error.load_field_no_result").to_string(),
                        detail: None,
                        retryable: true,
                    }),
                };
                let event = this
                    .controller
                    .dispatch(AppCommand::FinishOpenDesignTable {
                        result: result.clone(),
                    });
                this.apply_app_event(&event, cx);
                if let (AppEvent::TabOpened(tab_id), Ok(create)) = (&event, result)
                    && create.details_loading
                {
                    let Some(tab) = this.controller.state().tabs.iter().find(|tab| tab.id == *tab_id)
                    else {
                        return;
                    };
                    let TabKind::CreateTable(create) = &tab.kind else {
                        return;
                    };
                    let CreateTableMode::Design { object, .. } = &create.mode else {
                        return;
                    };
                    let object = object.clone();
                    this.spawn_design_table_details(*tab_id, object, cx);
                }
            },
            cx,
        );
        self._design_table_tasks.insert(key, task);
    }

    fn request_data_editor_refresh_for_object(
        &mut self,
        object: &ObjectPath,
        cx: &mut Context<Self>,
    ) {
        let tab_ids = self
            .controller
            .state()
            .tabs
            .iter()
            .filter_map(|tab| match &tab.kind {
                TabKind::DataEditor(editor) if editor.object == *object => Some(tab.id),
                _ => None,
            })
            .collect::<Vec<_>>();
        for tab_id in tab_ids {
            self.request_data_editor_refresh(tab_id, cx);
        }
    }

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

        self.show_message(tr("app.table.toast.refresh_requested").as_ref(), AppMessageKind::Success, cx);
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
        let needs_ddl = self
            .table_database_kind(&object_path)
            .is_some_and(copy_table_requires_source_ddl);
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
            // 默认 CONTINUE IDENTITY：不重置自增，可由用户在对话框显式改为 RESTART。
            restart_identity: false,
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

    /// PG 清空表时是否 RESTART IDENTITY；改动后重算 SQL 预览。
    fn set_danger_table_restart_identity(&mut self, restart: bool, cx: &mut Context<Self>) {
        if self._danger_table_task.is_some() {
            return;
        }
        if let Some(form) = &mut self.pending_danger_table_action {
            form.restart_identity = restart;
            form.error = None;
            cx.notify(); // notify 触发重渲染，render 处重算 SQL 预览（含 RESTART IDENTITY）。
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
            self.show_message(tr("app.table.toast.copying_structure").as_ref(), AppMessageKind::Warning, cx);
            return;
        }

        self.show_message(tr("app.table.toast.copying_structure").as_ref(), AppMessageKind::Success, cx);
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
                                tr("app.table.toast.structure_copied").replace("%{table}", &table_name),
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
            return Err(tr("app.error.connection_missing").to_string());
        };
        rename_table_sql_preview(
            database_kind,
            form.object_path.schema.as_deref(),
            &form.object_path.name,
            &form.new_name,
        )
    }

    fn copy_table_sql_for_form(&self, form: &PendingCopyTable) -> Result<String, String> {
        let Some(database_kind) = self.table_database_kind(&form.object_path) else {
            return Err(tr("app.error.connection_missing").to_string());
        };
        let source_ddl = if copy_table_requires_source_ddl(database_kind) {
            match &form.source_ddl {
                Some(Ok(ddl)) => Some(ddl.as_str()),
                Some(Err(message)) => return Err(message.clone()),
                None => return Err(tr("app.table.status.reading_structure").to_string()),
            }
        } else {
            None
        };
        copy_table_sql_preview_with_source_ddl(
            database_kind,
            form.object_path.schema.as_deref(),
            &form.object_path.name,
            &form.new_name,
            form.copy_data,
            source_ddl,
        )
    }

    fn danger_table_sql_for_form(&self, form: &PendingDangerTableAction) -> Result<String, String> {
        let Some(database_kind) = self.table_database_kind(&form.object_path) else {
            return Err(tr("app.error.connection_missing").to_string());
        };
        match form.action {
            DangerTableAction::Drop => drop_table_sql_preview(
                database_kind,
                form.object_path.kind,
                form.object_path.schema.as_deref(),
                &form.object_path.name,
                form.foreign_key_check,
            ),
            DangerTableAction::Truncate => truncate_table_sql_preview(
                database_kind,
                form.object_path.schema.as_deref(),
                &form.object_path.name,
                form.restart_identity,
                form.foreign_key_check,
            ),
        }
    }

    fn confirm_rename_table(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut form) = self.pending_rename_table.clone() else {
            return;
        };
        if self._rename_table_task.is_some() {
            self.show_message(tr("app.table.toast.renaming").as_ref(), AppMessageKind::Warning, cx);
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

        self.show_message(tr("app.table.toast.renaming").as_ref(), AppMessageKind::Success, cx);
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
                            this.refresh_database_children_for_table(&object, cx);
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
                            this.show_message(tr("app.table.toast.renamed").as_ref(), AppMessageKind::Success, cx);
                        }
                        AppEvent::Failed(error) => {
                            this.controller.merge_last_error_from(&controller);
                            if let Some(form) = &mut this.pending_rename_table {
                                form.error = Some(error.message.clone());
                            }
                            this.show_message(error.message, AppMessageKind::Error, cx);
                        }
                        _ => {
                            this.show_message(tr("app.table.error.rename_no_result").as_ref(), AppMessageKind::Error, cx);
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
            self.show_message(tr("app.table.toast.copying").as_ref(), AppMessageKind::Warning, cx);
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

        self.show_message(tr("app.table.toast.copying").as_ref(), AppMessageKind::Success, cx);
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
                            this.refresh_database_children_for_table(&object, cx);
                            this.show_message(
                                tr("app.table.toast.copied_as").replace("%{name}", &new_name),
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
                            this.show_message(tr("app.table.error.copy_no_result").as_ref(), AppMessageKind::Error, cx);
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
            form.error = Some(tr("app.table.error.confirm_required").to_string());
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
                            restart_identity: form.restart_identity,
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
                            this.show_message(tr("app.table.toast.deleted").as_ref(), AppMessageKind::Success, cx);
                        }
                        AppEvent::TableTruncated(object) => {
                            this.pending_danger_table_action = None;
                            this.refresh_open_data_tabs_for_table(&object, cx);
                            this.refresh_database_children_for_table(&object, cx);
                            this.show_message(tr("app.table.toast.truncated").as_ref(), AppMessageKind::Success, cx);
                        }
                        AppEvent::Failed(error) => {
                            this.controller.merge_last_error_from(&controller);
                            if let Some(form) = &mut this.pending_danger_table_action {
                                form.error = Some(error.message.clone());
                            }
                            this.show_message(error.message, AppMessageKind::Error, cx);
                        }
                        _ => {
                            this.show_message(tr("app.table.error.operation_no_result").as_ref(), AppMessageKind::Error, cx);
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
        // schema 分桶数据库只刷新表所在 schema，避免库级刷新清掉其他 schema。
        let (target, key) = if matches!(
            self.table_database_kind(object),
            Some(
                DatabaseKind::Postgres
                    | DatabaseKind::Kingbase
                    | DatabaseKind::Dameng
                    | DatabaseKind::SqlServer
                    | DatabaseKind::Oracle
            )
        ) {
            let (Some(database), Some(schema)) = (object.database.clone(), object.schema.clone())
            else {
                return;
            };
            (
                ObjectPath {
                    connection_id: object.connection_id,
                    database: Some(database.clone()),
                    schema: Some(schema.clone()),
                    name: schema.clone(),
                    kind: ObjectKind::Schema,
                },
                schema_tree_key(object.connection_id, &database, &schema),
            )
        } else {
            let Some(parent) = database_parent_path(object) else {
                return;
            };
            let database = parent
                .database
                .clone()
                .unwrap_or_else(|| parent.name.clone());
            (
                parent,
                database_tree_key(object.connection_id, &database),
            )
        };
        self.refresh_database_children(target, key, cx);
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
