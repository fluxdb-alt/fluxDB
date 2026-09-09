impl NavicatMain {
    fn show_connection_context_menu(
        &mut self,
        connection_id: ConnectionId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_rename_group.is_some() {
            self.confirm_rename_group(cx);
        }
        if self.pending_rename_table_folder.is_some() {
            self.confirm_rename_table_folder(cx);
        }
        self.connection_context_menu = Some(ConnectionContextMenu {
            connection_id,
            position,
            show_group_submenu: false,
        });
        self.database_context_menu = None;
        self.table_context_menu = None;
        self.table_group_context_menu = None;
        self.table_folder_context_menu = None;
        self.tab_context_menu = None;
        self.data_cell_context_menu = None;
        self.data_row_context_menu = None;
        self.tab_switcher = None;
        self.group_context_menu = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn show_database_context_menu(
        &mut self,
        mut menu: DatabaseContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_rename_table_folder.is_some() {
            self.confirm_rename_table_folder(cx);
        }
        menu.position = clamp_context_menu_position(menu.position, 238., 314., window);
        self.database_context_menu = Some(menu);
        self.connection_context_menu = None;
        self.table_context_menu = None;
        self.table_group_context_menu = None;
        self.table_folder_context_menu = None;
        self.tab_context_menu = None;
        self.data_cell_context_menu = None;
        self.data_row_context_menu = None;
        self.tab_switcher = None;
        self.group_context_menu = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn show_table_context_menu(
        &mut self,
        mut menu: TableContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_rename_table_folder.is_some() {
            self.confirm_rename_table_folder(cx);
        }
        menu.position = clamp_context_menu_position(menu.position, 212., table_context_menu_height(), window);
        self.table_context_menu = Some(menu);
        self.connection_context_menu = None;
        self.database_context_menu = None;
        self.table_group_context_menu = None;
        self.table_folder_context_menu = None;
        self.tab_context_menu = None;
        self.data_cell_context_menu = None;
        self.data_row_context_menu = None;
        self.tab_switcher = None;
        self.group_context_menu = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn show_table_group_context_menu(
        &mut self,
        mut menu: TableGroupContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_rename_table_folder.is_some() {
            self.confirm_rename_table_folder(cx);
        }
        menu.position = clamp_context_menu_position(menu.position, 188., context_menu_height(3., 0.), window);
        self.table_group_context_menu = Some(menu);
        self.connection_context_menu = None;
        self.database_context_menu = None;
        self.table_context_menu = None;
        self.table_folder_context_menu = None;
        self.tab_context_menu = None;
        self.data_cell_context_menu = None;
        self.data_row_context_menu = None;
        self.tab_switcher = None;
        self.group_context_menu = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn show_table_folder_context_menu(
        &mut self,
        mut menu: TableFolderContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_rename_table_folder.is_some() {
            self.confirm_rename_table_folder(cx);
        }
        menu.position = clamp_context_menu_position(menu.position, 188., context_menu_height(4., 0.), window);
        self.table_folder_context_menu = Some(menu);
        self.connection_context_menu = None;
        self.database_context_menu = None;
        self.table_context_menu = None;
        self.table_group_context_menu = None;
        self.tab_context_menu = None;
        self.data_cell_context_menu = None;
        self.data_row_context_menu = None;
        self.tab_switcher = None;
        self.group_context_menu = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn show_tab_context_menu(
        &mut self,
        tab_id: TabId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_rename_table_folder.is_some() {
            self.confirm_rename_table_folder(cx);
        }
        self.tab_context_menu = Some(TabContextMenu { tab_id, position });
        self.connection_context_menu = None;
        self.database_context_menu = None;
        self.table_context_menu = None;
        self.table_group_context_menu = None;
        self.table_folder_context_menu = None;
        self.data_cell_context_menu = None;
        self.data_row_context_menu = None;
        self.tab_switcher = None;
        self.group_context_menu = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn show_group_context_menu(
        &mut self,
        group_id: ConnectionGroupId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_rename_group.is_some() {
            self.confirm_rename_group(cx);
        }
        if self.pending_rename_table_folder.is_some() {
            self.confirm_rename_table_folder(cx);
        }
        self.group_context_menu = Some(GroupContextMenu { group_id, position });
        self.connection_context_menu = None;
        self.database_context_menu = None;
        self.table_context_menu = None;
        self.table_group_context_menu = None;
        self.table_folder_context_menu = None;
        self.tab_context_menu = None;
        self.data_cell_context_menu = None;
        self.data_row_context_menu = None;
        self.tab_switcher = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn show_data_cell_context_menu(
        &mut self,
        mut menu: DataCellContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_rename_table_folder.is_some() {
            self.confirm_rename_table_folder(cx);
        }
        menu.position = clamp_context_menu_position(menu.position, 238., data_cell_context_menu_height(&menu), window);
        self.data_cell_context_menu = Some(menu);
        self.connection_context_menu = None;
        self.database_context_menu = None;
        self.table_context_menu = None;
        self.table_group_context_menu = None;
        self.table_folder_context_menu = None;
        self.tab_context_menu = None;
        self.data_row_context_menu = None;
        self.group_context_menu = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn show_data_row_context_menu(
        &mut self,
        mut menu: DataRowContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_rename_table_folder.is_some() {
            self.confirm_rename_table_folder(cx);
        }
        menu.position = clamp_context_menu_position(menu.position, 196., data_row_context_menu_height(&menu), window);
        self.data_row_context_menu = Some(menu);
        self.connection_context_menu = None;
        self.database_context_menu = None;
        self.table_context_menu = None;
        self.table_group_context_menu = None;
        self.table_folder_context_menu = None;
        self.tab_context_menu = None;
        self.data_cell_context_menu = None;
        self.group_context_menu = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn set_data_row_context_submenu(
        &mut self,
        tab_id: TabId,
        submenu: Option<DataRowContextSubmenu>,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = &mut self.data_row_context_menu else {
            return;
        };
        if menu.tab_id == tab_id && menu.submenu != submenu {
            menu.submenu = submenu;
            cx.notify();
        }
    }

    fn set_table_context_submenu(
        &mut self,
        object_path: ObjectPath,
        submenu: Option<TableContextSubmenu>,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = &mut self.table_context_menu else {
            return;
        };
        if menu.object_path == object_path && menu.submenu != submenu {
            menu.submenu = submenu;
            cx.notify();
        }
    }

    fn handle_table_menu_action(
        &mut self,
        action: TableMenuAction,
        object_path: ObjectPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.table_context_menu = None;
        match action {
            TableMenuAction::TogglePin => {
                let table_key = table_tree_key(&object_path);
                if !self.pinned_tables.remove(&table_key) {
                    self.pinned_tables.insert(table_key);
                }
            }
            TableMenuAction::CopyName => {
                cx.write_to_clipboard(ClipboardItem::new_string(object_path.name));
                self.show_message("已复制表名", AppMessageKind::Success, cx);
            }
            TableMenuAction::ViewData => {
                self.dispatch(AppCommand::OpenDataEditor(object_path), cx);
            }
            TableMenuAction::Design => {
                self.dispatch(AppCommand::OpenDesignTable(object_path), cx);
            }
            TableMenuAction::NewTable => {
                self.dispatch(
                    AppCommand::OpenCreateTable {
                        connection_id: object_path.connection_id,
                        database: object_path.database.clone(),
                    },
                    cx,
                );
            }
            TableMenuAction::Refresh => {
                self.refresh_table_object(&object_path, cx);
            }
            TableMenuAction::Rename => {
                self.open_rename_table_modal(object_path, window, cx);
            }
            TableMenuAction::CopyTable => {
                self.open_copy_table_modal(object_path, window, cx);
            }
            TableMenuAction::ExportData => {
                self.open_table_data_export_from_object(object_path, cx);
            }
            TableMenuAction::CopyStructure => {
                self.copy_table_structure(object_path, cx);
            }
            TableMenuAction::Backup => {
                self.show_backup_modal_preselect(
                    Some(object_path.connection_id),
                    object_path.database.clone(),
                    Some(&object_path.name),
                    window,
                    cx,
                );
            }
            TableMenuAction::Drop => {
                self.open_danger_table_modal(object_path, DangerTableAction::Drop, window, cx);
            }
            TableMenuAction::Truncate => {
                self.open_danger_table_modal(object_path, DangerTableAction::Truncate, window, cx);
            }
        }
        cx.notify();
    }

    fn handle_table_group_menu_action(
        &mut self,
        action: TableGroupMenuAction,
        menu: TableGroupContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.table_group_context_menu = None;
        let database_key = database_tree_key(menu.connection_id, &menu.database);
        match action {
            TableGroupMenuAction::NewTable => {
                self.dispatch(
                    AppCommand::OpenCreateTable {
                        connection_id: menu.connection_id,
                        database: Some(menu.database),
                    },
                    cx,
                );
            }
            TableGroupMenuAction::NewGroup => {
                let parent_key = table_folder_parent_key(menu.connection_id, &menu.database);
                let group_key =
                    object_group_tree_key(menu.connection_id, &menu.database, ObjectGroup::Tables);
                let folder_name = next_table_folder_name(
                    self.table_folders
                        .get(&parent_key)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                );
                self.expanded_object_groups.insert(group_key, true);
                self.expanded_object_groups
                    .insert(table_folder_tree_key(&parent_key, &folder_name), true);
                self.table_folders
                    .entry(parent_key.clone())
                    .or_default()
                    .push(folder_name.clone());
                self.persist_sidebar_layout();
                self.start_rename_table_folder(parent_key, folder_name, window, cx);
            }
            TableGroupMenuAction::Refresh => {
                self.loaded_database_children.remove(&database_key);
                self.load_database_children(menu.database_path, database_key, cx);
                return;
            }
        }
        cx.notify();
    }

    fn handle_table_folder_menu_action(
        &mut self,
        action: TableFolderMenuAction,
        menu: TableFolderContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.table_folder_context_menu = None;
        match action {
            TableFolderMenuAction::MoveUp => {
                self.move_table_folder(&menu.parent_key, &menu.name, -1, cx);
            }
            TableFolderMenuAction::MoveDown => {
                self.move_table_folder(&menu.parent_key, &menu.name, 1, cx);
            }
            TableFolderMenuAction::Rename => {
                self.start_rename_table_folder(menu.parent_key, menu.name, window, cx);
            }
            TableFolderMenuAction::Delete => {
                self.delete_table_folder(&menu.parent_key, &menu.name, cx);
            }
        }
        cx.notify();
    }

    fn move_table_folder(
        &mut self,
        parent_key: &str,
        name: &str,
        direction: isize,
        cx: &mut Context<Self>,
    ) {
        let Some(folders) = self.table_folders.get_mut(parent_key) else {
            return;
        };
        move_table_folder_name(folders, name, direction);
        self.persist_sidebar_layout();
        cx.notify();
    }

    fn select_table_folder(&mut self, parent_key: String, name: String, cx: &mut Context<Self>) {
        self.selected_table_folder = Some((parent_key, name));
        cx.notify();
    }

    fn assign_table_to_folder(
        &mut self,
        object_path: ObjectPath,
        parent_key: String,
        folder: String,
        cx: &mut Context<Self>,
    ) {
        self.table_context_menu = None;
        self.table_folder_assignments
            .insert(table_tree_key(&object_path), (parent_key, folder));
        self.persist_sidebar_layout();
        cx.notify();
    }

    fn start_rename_table_folder(
        &mut self,
        parent_key: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_table_folder = Some((parent_key.clone(), name.clone()));
        self.pending_rename_table_folder = Some(PendingRenameTableFolder {
            parent_key,
            original_name: name.clone(),
            name: name.clone(),
        });
        self.table_folder_rename_input.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn cancel_rename_table_folder(&mut self, cx: &mut Context<Self>) {
        self.pending_rename_table_folder = None;
        cx.notify();
    }

    fn confirm_rename_table_folder(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_rename_table_folder.take() else {
            return;
        };
        let name = pending.name.trim();
        if name.is_empty() || name == pending.original_name {
            cx.notify();
            return;
        }
        let Some(folders) = self.table_folders.get_mut(&pending.parent_key) else {
            cx.notify();
            return;
        };
        if folders
            .iter()
            .any(|folder| folder != &pending.original_name && folder == name)
        {
            self.pending_rename_table_folder = Some(pending);
            self.show_message("分组名称已存在", AppMessageKind::Warning, cx);
            return;
        }
        if let Some(folder) = folders
            .iter_mut()
            .find(|folder| **folder == pending.original_name)
        {
            *folder = name.to_string();
            let new_name = name.to_string();
            let old_expanded_key = table_folder_tree_key(&pending.parent_key, &pending.original_name);
            let new_expanded_key = table_folder_tree_key(&pending.parent_key, &new_name);
            if let Some(expanded) = self.expanded_object_groups.remove(&old_expanded_key) {
                self.expanded_object_groups.insert(new_expanded_key, expanded);
            }
            for (parent_key, assigned_folder) in self.table_folder_assignments.values_mut() {
                if parent_key == &pending.parent_key && assigned_folder == &pending.original_name {
                    *assigned_folder = new_name.clone();
                }
            }
            self.selected_table_folder = Some((pending.parent_key, new_name));
            self.persist_sidebar_layout();
        }
        cx.notify();
    }

    fn delete_table_folder(&mut self, parent_key: &str, name: &str, cx: &mut Context<Self>) {
        if let Some(folders) = self.table_folders.get_mut(parent_key) {
            folders.retain(|folder| folder != name);
            if folders.is_empty() {
                self.table_folders.remove(parent_key);
            }
        }
        self.expanded_object_groups
            .remove(&table_folder_tree_key(parent_key, name));
        if self
            .selected_table_folder
            .as_ref()
            .is_some_and(|(selected_parent, selected_name)| {
                selected_parent == parent_key && selected_name == name
            })
        {
            self.selected_table_folder = None;
        }
        if self
            .pending_rename_table_folder
            .as_ref()
            .is_some_and(|pending| pending.parent_key == parent_key && pending.original_name == name)
        {
            self.pending_rename_table_folder = None;
        }
        self.table_folder_assignments.retain(
            |_, (assigned_parent_key, assigned_folder)| {
                !(assigned_parent_key == parent_key && assigned_folder == name)
            },
        );
        self.persist_sidebar_layout();
        cx.notify();
    }

    fn row_fields_for_tab(
        &self,
        tab_id: TabId,
        query_result_page_index: Option<usize>,
        source_row: usize,
    ) -> Option<DataRowSnapshot> {
        self.controller.state().tabs.iter().find_map(|tab| {
            if tab.id != tab_id {
                return None;
            }
            match &tab.kind {
                TabKind::DataEditor(editor) => {
                    let page = editor.page.as_ref()?;
                    let fields = row_fields_for_page(page, source_row)?;
                    Some(DataRowSnapshot {
                        object: Some(editor.object.clone()),
                        display_name: editor.object.name.clone(),
                        fields,
                    })
                }
                TabKind::QueryEditor(editor) => {
                    let page_index = query_result_page_index.or(editor.active_result_editor)?;
                    let editable = editor.result_editors.get(&page_index);
                    let page = editable
                        .and_then(|editor| editor.page.as_ref())
                        .or_else(|| editor.results.get(page_index))?;
                    let fields = row_fields_for_page(page, source_row)?;
                    Some(DataRowSnapshot {
                        object: editable.map(|editor| editor.object.clone()),
                        display_name: editable
                            .map(|editor| editor.object.name.clone())
                            .unwrap_or_else(|| format!("查询结果 {}", page_index + 1)),
                        fields,
                    })
                }
                _ => None,
            }
        })
    }

    fn selected_source_rows_for_tab(&self, tab_id: TabId, cx: &App) -> Vec<usize> {
        let Some(table_state) = self.data_table_states.get(&tab_id) else {
            return Vec::new();
        };
        let table = table_state.read(cx);
        let delegate = table.delegate();
        delegate
            .effective_selected_rows()
            .iter()
            .filter_map(|row| delegate.source_row_indexes.get(*row).copied())
            .collect()
    }

    fn row_copy_source_rows_for_tab(
        &self,
        tab_id: TabId,
        fallback_source_row: usize,
        cx: &App,
    ) -> Vec<usize> {
        let selected_rows = self.selected_source_rows_for_tab(tab_id, cx);
        if selected_rows.is_empty() {
            vec![fallback_source_row]
        } else {
            selected_rows
        }
    }

    fn open_data_row_viewer(
        &mut self,
        tab_id: TabId,
        query_result_page_index: Option<usize>,
        source_row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.data_row_viewer = Some(DataRowViewer {
            tab_id,
            source_row,
            query_result_page_index,
        });
        self.data_row_context_menu = None;
        self.row_detail_search.clear();
        self.row_detail_search_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        cx.notify();
    }

    fn close_data_row_viewer(&mut self, cx: &mut Context<Self>) {
        self.data_row_viewer = None;
        cx.notify();
    }

    fn copy_data_row_text(
        &mut self,
        tab_id: TabId,
        query_result_page_index: Option<usize>,
        source_row: usize,
        kind: DataRowCopyKind,
        cx: &mut Context<Self>,
    ) {
        let source_rows = self.row_copy_source_rows_for_tab(tab_id, source_row, cx);
        let row_snapshots = source_rows
            .iter()
            .filter_map(|source_row| {
                self.row_fields_for_tab(tab_id, query_result_page_index, *source_row)
            })
            .collect::<Vec<_>>();
        if row_snapshots.is_empty() {
            return;
        };
        let object = row_snapshots[0].object.clone();
        let text = match kind {
            DataRowCopyKind::Json if row_snapshots.len() > 1 => row_json_array_text(
                row_snapshots
                    .iter()
                    .map(|snapshot| snapshot.fields.as_slice())
                    .collect::<Vec<_>>()
                    .as_slice(),
            ),
            DataRowCopyKind::Json => row_json_text(row_snapshots[0].fields.as_slice()),
            DataRowCopyKind::Insert => {
                let Some(object) = object.as_ref() else {
                    self.show_message("当前结果没有表对象信息，无法复制 SQL", AppMessageKind::Warning, cx);
                    return;
                };
                row_snapshots
                    .iter()
                    .map(|snapshot| row_insert_sql(object, snapshot.fields.as_slice(), false))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            DataRowCopyKind::InsertWithoutPrimaryKey => {
                let Some(object) = object.as_ref() else {
                    self.show_message("当前结果没有表对象信息，无法复制 SQL", AppMessageKind::Warning, cx);
                    return;
                };
                row_snapshots
                    .iter()
                    .map(|snapshot| row_insert_sql(object, snapshot.fields.as_slice(), true))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            DataRowCopyKind::Update => {
                let Some(object) = object.as_ref() else {
                    self.show_message("当前结果没有表对象信息，无法复制 SQL", AppMessageKind::Warning, cx);
                    return;
                };
                row_snapshots
                    .iter()
                    .map(|snapshot| row_update_sql(object, snapshot.fields.as_slice()))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            DataRowCopyKind::Tsv if row_snapshots.len() > 1 => row_tsv_rows_text(
                row_snapshots
                    .iter()
                    .map(|snapshot| snapshot.fields.as_slice())
                    .collect::<Vec<_>>()
                    .as_slice(),
            ),
            DataRowCopyKind::Tsv => row_tsv_text(row_snapshots[0].fields.as_slice()),
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.data_row_context_menu = None;
        if row_snapshots.len() > 1 {
            self.show_message(
                format!("已复制 {} 行", row_snapshots.len()),
                AppMessageKind::Success,
                cx,
            );
        } else {
            self.show_message("已复制", AppMessageKind::Success, cx);
        }
    }

    fn data_table_selection_export(
        &self,
        tab_id: TabId,
        cx: &App,
    ) -> Option<DataTableSelectionExport> {
        self.data_table_states
            .get(&tab_id)
            .and_then(|table_state| table_state.read(cx).delegate().selection_export())
    }

    fn copy_data_table_selection(&mut self, tab_id: TabId, cx: &mut Context<Self>) -> bool {
        let Some(export) = self.data_table_selection_export(tab_id, cx) else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(export.text));
        self.data_cell_context_menu = None;
        self.data_row_context_menu = None;
        let message = match export.kind {
            DataTableSelectionKind::Rows => format!("已复制 {} 行", export.rows),
            DataTableSelectionKind::Cells => format!("已复制 {} 个单元格", export.cells),
        };
        self.show_message(message, AppMessageKind::Success, cx);
        true
    }

    fn export_data_table_selection(&mut self, tab_id: TabId, cx: &mut Context<Self>) -> bool {
        let Some(export) = self.data_table_selection_export(tab_id, cx) else {
            return false;
        };
        let timestamp = Local::now().format("%Y%m%d-%H%M%S");
        let suggested_name = format!("gdb-selection-{}-{timestamp}.tsv", tab_id.0);
        let receiver =
            cx.prompt_for_new_path(&default_data_export_directory(), Some(&suggested_name));
        self.data_cell_context_menu = None;
        self.data_row_context_menu = None;
        self.spawn_data_selection_export_task(receiver, export, cx);
        true
    }

    fn export_data_rows(
        &mut self,
        tab_id: TabId,
        query_result_page_index: Option<usize>,
        source_row: usize,
        format: DataRowExportFormat,
        cx: &mut Context<Self>,
    ) -> bool {
        let source_rows = self.row_copy_source_rows_for_tab(tab_id, source_row, cx);
        let row_snapshots = source_rows
            .iter()
            .filter_map(|source_row| {
                self.row_fields_for_tab(tab_id, query_result_page_index, *source_row)
            })
            .collect::<Vec<_>>();
        if row_snapshots.is_empty() {
            return false;
        }

        let object = row_snapshots[0].object.clone();
        if format == DataRowExportFormat::SqlInsert && object.is_none() {
            self.show_message("当前结果没有表对象信息，无法导出 SQL", AppMessageKind::Warning, cx);
            return false;
        }
        let suggested_name = object
            .as_ref()
            .map(|object| data_row_export_suggested_name(object, format))
            .unwrap_or_else(|| {
                format!(
                    "gdb-{}.{}",
                    safe_data_export_filename_segment(&row_snapshots[0].display_name),
                    format.extension()
                )
            });
        let rows = row_snapshots
            .into_iter()
            .map(|snapshot| snapshot.fields)
            .collect::<Vec<_>>();
        let receiver =
            cx.prompt_for_new_path(&default_data_export_directory(), Some(&suggested_name));
        self.data_cell_context_menu = None;
        self.data_row_context_menu = None;
        self.spawn_data_row_export_task(receiver, object, rows, format, cx);
        true
    }

    fn next_data_export_task_id(&mut self) -> u64 {
        self.data_export_task_seq = self.data_export_task_seq.wrapping_add(1);
        self.data_export_task_seq
    }

    fn spawn_data_row_export_task(
        &mut self,
        receiver: futures::channel::oneshot::Receiver<anyhow::Result<Option<PathBuf>>>,
        object: Option<ObjectPath>,
        rows: Vec<Vec<RowFieldSnapshot>>,
        format: DataRowExportFormat,
        cx: &mut Context<Self>,
    ) {
        let task_id = self.next_data_export_task_id();
        let row_count = rows.len();
        self.show_message(
            format!("选择保存位置后导出 {} 行", row_count),
            AppMessageKind::Success,
            cx,
        );
        let task = cx.spawn(async move |view, cx| {
            let selected = receiver.await;
            let path = match selected {
                Ok(Ok(Some(path))) => Some(safe_data_export_path(path, format)),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.show_message(
                                format!("选择保存位置失败：{error}"),
                                AppMessageKind::Error,
                                cx,
                            );
                            this._data_export_tasks.remove(&task_id);
                            cx.notify();
                        });
                    });
                    None
                }
                Err(error) => {
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.show_message(
                                format!("选择保存位置失败：{error}"),
                                AppMessageKind::Error,
                                cx,
                            );
                            this._data_export_tasks.remove(&task_id);
                            cx.notify();
                        });
                    });
                    None
                }
            };
            let Some(path) = path else {
                let _ = cx.update(|cx| {
                    let Some(view) = view.upgrade() else {
                        return;
                    };
                    view.update(cx, |this, cx| {
                        this._data_export_tasks.remove(&task_id);
                        cx.notify();
                    });
                });
                return;
            };

            let result = cx
                .background_spawn({
                    let path = path.clone();
                    let object = object.clone();
                    async move {
                        write_data_row_export_file(
                            &path,
                            format,
                            object.as_ref(),
                            rows.as_slice(),
                        )
                            .map(|_| path)
                    }
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._data_export_tasks.remove(&task_id);
                    match result {
                        Ok(path) => {
                            this.show_message(
                                format!(
                                    "已导出 {} 行为 {}：{}",
                                    row_count,
                                    format.label(),
                                    path.display()
                                ),
                                AppMessageKind::Success,
                                cx,
                            );
                        }
                        Err(error) => {
                            this.show_message(format!("导出失败：{error}"), AppMessageKind::Error, cx);
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._data_export_tasks.insert(task_id, task);
        cx.notify();
    }

    fn spawn_data_selection_export_task(
        &mut self,
        receiver: futures::channel::oneshot::Receiver<anyhow::Result<Option<PathBuf>>>,
        export: DataTableSelectionExport,
        cx: &mut Context<Self>,
    ) {
        let task_id = self.next_data_export_task_id();
        let label = match export.kind {
            DataTableSelectionKind::Rows => format!("{} 行", export.rows),
            DataTableSelectionKind::Cells => format!("{} 个单元格", export.cells),
        };
        self.show_message(
            format!("选择保存位置后导出 {label}"),
            AppMessageKind::Success,
            cx,
        );
        let task = cx.spawn(async move |view, cx| {
            let selected = receiver.await;
            let path = match selected {
                Ok(Ok(Some(path))) => Some(safe_data_export_path_with_extension(path, "tsv")),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.show_message(
                                format!("选择保存位置失败：{error}"),
                                AppMessageKind::Error,
                                cx,
                            );
                            this._data_export_tasks.remove(&task_id);
                            cx.notify();
                        });
                    });
                    None
                }
                Err(error) => {
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.show_message(
                                format!("选择保存位置失败：{error}"),
                                AppMessageKind::Error,
                                cx,
                            );
                            this._data_export_tasks.remove(&task_id);
                            cx.notify();
                        });
                    });
                    None
                }
            };
            let Some(path) = path else {
                let _ = cx.update(|cx| {
                    let Some(view) = view.upgrade() else {
                        return;
                    };
                    view.update(cx, |this, cx| {
                        this._data_export_tasks.remove(&task_id);
                        cx.notify();
                    });
                });
                return;
            };

            let result = cx
                .background_spawn({
                    let path = path.clone();
                    async move { write_data_selection_export_file(&path, &export).map(|_| path) }
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._data_export_tasks.remove(&task_id);
                    match result {
                        Ok(path) => {
                            this.show_message(
                                format!("已导出 TSV：{}", path.display()),
                                AppMessageKind::Success,
                                cx,
                            );
                        }
                        Err(error) => {
                            this.show_message(format!("导出失败：{error}"), AppMessageKind::Error, cx);
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._data_export_tasks.insert(task_id, task);
        cx.notify();
    }

    fn insert_data_row(
        &mut self,
        tab_id: TabId,
        query_result_page_index: Option<usize>,
        after_row: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.data_row_context_menu = None;
        self.dispatch(
            AppCommand::InsertDataRow {
                tab_id,
                result_index: query_result_page_index,
                after_row,
            },
            cx,
        );
        self.refresh_active_data_table(tab_id, cx);
    }

    fn clone_data_row(
        &mut self,
        tab_id: TabId,
        query_result_page_index: Option<usize>,
        source_row: usize,
        cx: &mut Context<Self>,
    ) {
        self.data_row_context_menu = None;
        self.dispatch(
            AppCommand::CloneDataRow {
                tab_id,
                result_index: query_result_page_index,
                row: source_row,
                after_row: Some(source_row),
            },
            cx,
        );
        self.refresh_active_data_table(tab_id, cx);
    }

    fn delete_data_row(
        &mut self,
        tab_id: TabId,
        query_result_page_index: Option<usize>,
        source_row: usize,
        cx: &mut Context<Self>,
    ) {
        let source_rows = self.row_copy_source_rows_for_tab(tab_id, source_row, cx);
        self.data_row_context_menu = None;
        for row in source_rows {
            self.dispatch(
                AppCommand::DeleteDataRow {
                    tab_id,
                    result_index: query_result_page_index,
                    row,
                },
                cx,
            );
        }
        self.refresh_active_data_table(tab_id, cx);
    }

    fn set_data_cell_context_submenu(
        &mut self,
        tab_id: TabId,
        submenu: Option<DataCellContextSubmenu>,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = &mut self.data_cell_context_menu else {
            return;
        };
        if menu.tab_id == tab_id && menu.submenu != submenu {
            menu.submenu = submenu;
            cx.notify();
        }
    }

    fn set_data_cell_value(
        &mut self,
        menu: &DataCellContextMenu,
        value: CellValue,
        cx: &mut Context<Self>,
    ) {
        if data_type_is_binary(menu.type_name.as_str()) {
            self.data_cell_context_menu = None;
            self.show_message("二进制字段不能修改", AppMessageKind::Warning, cx);
            return;
        }
        self.dispatch(
            AppCommand::EditDataCell {
                tab_id: menu.tab_id,
                row: menu.source_row,
                column: menu.source_col,
                value,
            },
            cx,
        );
        self.refresh_active_data_table(menu.tab_id, cx);
        self.data_cell_context_menu = None;
    }

    fn request_delete_data_cell_row(&mut self, menu: DataCellContextMenu, cx: &mut Context<Self>) {
        self.pending_delete_data_row = Some(menu);
        self.data_cell_context_menu = None;
        cx.notify();
    }

    fn confirm_delete_data_cell_row(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = self.pending_delete_data_row.take() else {
            return;
        };
        let source_rows = self.row_copy_source_rows_for_tab(menu.tab_id, menu.source_row, cx);
        for row in source_rows {
            self.dispatch(
                AppCommand::DeleteDataRow {
                    tab_id: menu.tab_id,
                    result_index: menu.query_result_page_index,
                    row,
                },
                cx,
            );
        }
        self.refresh_active_data_table(menu.tab_id, cx);
        cx.notify();
    }

    fn cancel_delete_data_cell_row(&mut self, cx: &mut Context<Self>) {
        self.pending_delete_data_row = None;
        cx.notify();
    }

}

fn clamp_context_menu_position(
    position: Point<Pixels>,
    width: f32,
    height: f32,
    window: &Window,
) -> Point<Pixels> {
    let viewport = window.viewport_size();
    let margin = 8.;
    let max_x = (f32::from(viewport.width) - width - margin).max(margin);
    let max_y = (f32::from(viewport.height) - height - margin).max(margin);

    point(
        px(f32::from(position.x).clamp(margin, max_x)),
        px(f32::from(position.y).clamp(margin, max_y)),
    )
}

fn data_row_context_menu_height(menu: &DataRowContextMenu) -> f32 {
    let items = if menu.rows_editable { 6. } else { 3. };
    let separators = if menu.rows_editable { 3. } else { 2. };
    context_menu_height(items, separators)
}

fn table_context_menu_height() -> f32 {
    context_menu_height(14., 3.)
}

fn data_cell_context_menu_height(menu: &DataCellContextMenu) -> f32 {
    let mut items = 11.;
    let mut separators = 5.;
    if menu.selection_copy_label.is_some() {
        items += 1.;
        separators += 1.;
    }
    if menu.selection_export_label.is_some() {
        items += 1.;
        separators += 1.;
    }
    context_menu_height(items, separators)
}

fn context_menu_height(items: f32, separators: f32) -> f32 {
    items * 26. + separators * 5. + 8.
}
