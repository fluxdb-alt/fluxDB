impl NavicatMain {
    fn save_active_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((tab_id, _, _, text)) = self.active_query_snapshot(cx) else {
            return;
        };
        if text.trim().is_empty() {
            self.show_message("没有可保存的 SQL", AppMessageKind::Warning, cx);
            return;
        }

        if let Some(target) = self.query_save_targets.get(&tab_id).cloned() {
            self.save_query_to_target(tab_id, target, cx);
            return;
        }

        self.pending_query_save = Some(tab_id);
        self.query_save_name_input
            .update(cx, |input, cx| input.set_value(default_saved_query_name(&text), window, cx));
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn cancel_query_save_modal(&mut self, cx: &mut Context<Self>) {
        if self.pending_query_save.take().is_some()
            || self.pending_connection_query_save.take().is_some()
        {
            cx.notify();
        }
    }

    fn choose_query_save_local(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.pending_query_save = None;
        let Some((_, _, _, text)) = self.active_query_snapshot(cx) else {
            return;
        };
        let suggested = default_sql_file_name(&text);
        let receiver = cx.prompt_for_new_path(&default_data_export_directory(), Some(&suggested));
        let task = cx.spawn(async move |view, cx| {
            let selected = receiver.await;
            let path = match selected {
                Ok(Ok(Some(path))) => Some(sql_path_with_extension(path)),
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
                        });
                    });
                    None
                }
            };
            let Some(path) = path else {
                return;
            };

            let result = cx.background_spawn(save_query_file(path, text)).await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| match result {
                    Ok(path) => {
                        this.query_save_targets
                            .insert(tab_id, QuerySaveTarget::Local(path.clone()));
                        this.mark_query_saved(
                            tab_id,
                            local_query_title(&path),
                            QueryOrigin::File { path },
                            cx,
                        );
                        this.show_message("SQL 已保存到本地", AppMessageKind::Success, cx);
                    }
                    Err(error) => {
                        this.show_message(format!("保存失败：{error}"), AppMessageKind::Error, cx);
                    }
                });
            });
        });
        self._file_picker_task = Some(task);
        cx.notify();
    }

    fn choose_query_save_connection(&mut self, tab_id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_query_save = None;
        self.pending_connection_query_save = Some(tab_id);
        if let Some((_, _, _, text)) = self.active_query_snapshot(cx) {
            self.query_save_name_input.update(cx, |input, cx| {
                input.set_value(default_saved_query_name(&text), window, cx);
                input.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn confirm_query_save_connection(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let name = self.query_save_name_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.show_message("请输入查询名称", AppMessageKind::Warning, cx);
            return;
        }
        let Some((_, connection_id, database, text)) = self.active_query_snapshot(cx) else {
            return;
        };

        let id = self
            .query_save_targets
            .get(&tab_id)
            .and_then(|target| match target {
                QuerySaveTarget::Connection(id) => Some(*id),
                QuerySaveTarget::Local(_) => None,
            })
            .or_else(|| {
                self.saved_queries
                    .iter()
                    .find(|query| {
                        query.connection_id == connection_id
                            && query.database == database
                            && query.name == name
                    })
                    .map(|query| query.id)
            })
            .unwrap_or_else(|| self.saved_queries.iter().map(|query| query.id).max().unwrap_or(0) + 1);

        if let Some(query) = self.saved_queries.iter_mut().find(|query| query.id == id) {
            query.name = name.clone();
            query.text = text;
            query.database = database;
            query.connection_id = connection_id;
        } else {
            self.saved_queries.push(SavedQuery {
                id,
                connection_id,
                database,
                name: name.clone(),
                text,
            });
        }

        match self.storage.save_saved_queries(&self.saved_queries) {
            Ok(()) => {
                self.query_save_targets
                    .insert(tab_id, QuerySaveTarget::Connection(id));
                self.pending_connection_query_save = None;
                self.mark_query_saved(
                    tab_id,
                    name,
                    QueryOrigin::Connection { query_id: id },
                    cx,
                );
                self.show_message("SQL 已保存到连接中", AppMessageKind::Success, cx);
                cx.notify();
            }
            Err(error) => {
                self.show_message(format!("保存失败：{error}"), AppMessageKind::Error, cx);
            }
        }
    }

    fn open_saved_query(&mut self, query_id: u64, cx: &mut Context<Self>) {
        if let Some(tab_id) = saved_query_tab_id(self.controller.state(), query_id) {
            if self.controller.state().active_tab != Some(tab_id) {
                self.dispatch(AppCommand::ActivateTab(tab_id), cx);
            }
            return;
        }

        let Some(query) = self
            .saved_queries
            .iter()
            .find(|query| query.id == query_id)
            .cloned()
        else {
            return;
        };
        let event = self.controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: query.connection_id,
            database: query.database.clone(),
        });
        self.apply_app_event(&event, cx);
        if let AppEvent::TabOpened(tab_id) = event {
            self.dispatch(AppCommand::UpdateQueryText {
                tab_id,
                text: query.text.clone(),
            }, cx);
            if let Some(editor) = self.query_editors.get(&tab_id) {
                editor.update(cx, |editor, cx| editor.sync_text_silent(&query.text, cx));
            }
            self.query_save_targets
                .insert(tab_id, QuerySaveTarget::Connection(query.id));
            self.mark_query_saved(
                tab_id,
                query.name,
                QueryOrigin::Connection { query_id: query.id },
                cx,
            );
        }
        cx.notify();
    }

    fn save_query_to_target(
        &mut self,
        tab_id: TabId,
        target: QuerySaveTarget,
        cx: &mut Context<Self>,
    ) {
        match target {
            QuerySaveTarget::Local(path) => {
                let Some((_, _, _, text)) = self.active_query_snapshot(cx) else {
                    return;
                };
                self._file_picker_task = Some(cx.spawn(async move |view, cx| {
                    let result = cx.background_spawn(save_query_file(path, text)).await;
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| match result {
                            Ok(path) => {
                                this.mark_query_saved(
                                    tab_id,
                                    local_query_title(&path),
                                    QueryOrigin::File { path },
                                    cx,
                                );
                                this.show_message("SQL 已保存", AppMessageKind::Success, cx);
                            }
                            Err(error) => {
                                this.show_message(
                                    format!("保存失败：{error}"),
                                    AppMessageKind::Error,
                                    cx,
                                );
                            }
                        });
                    });
                }));
            }
            QuerySaveTarget::Connection(id) => {
                let Some((_, connection_id, database, text)) = self.active_query_snapshot(cx) else {
                    return;
                };
                if let Some(query) = self.saved_queries.iter_mut().find(|query| query.id == id) {
                    query.connection_id = connection_id;
                    query.database = database;
                    query.text = text;
                    let title = query.name.clone();
                    match self.storage.save_saved_queries(&self.saved_queries) {
                        Ok(()) => {
                            self.mark_query_saved(
                                tab_id,
                                title,
                                QueryOrigin::Connection { query_id: id },
                                cx,
                            );
                            self.show_message("SQL 已保存", AppMessageKind::Success, cx);
                        }
                        Err(error) => {
                            self.show_message(
                                format!("保存失败：{error}"),
                                AppMessageKind::Error,
                                cx,
                            );
                        }
                    }
                } else {
                    self.pending_connection_query_save = Some(tab_id);
                    cx.notify();
                }
            }
        }
    }

    fn active_query_snapshot(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(TabId, ConnectionId, Option<String>, String)> {
        let tab_id = self.controller.state().active_tab?;
        if let Some(sql_editor) = self.query_editors.get(&tab_id).cloned() {
            let text = sql_editor.read(cx).text();
            let current = self
                .controller
                .state()
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .and_then(|tab| match &tab.kind {
                    TabKind::QueryEditor(editor) => Some(editor.text.as_str()),
                    _ => None,
                });
            if current != Some(text.as_str()) {
                self.dispatch(AppCommand::UpdateQueryText { tab_id, text }, cx);
            }
        }

        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::QueryEditor(editor) => Some((
                    tab_id,
                    editor.connection_id,
                    editor.database.clone(),
                    editor.text.clone(),
                )),
                _ => None,
            })
    }

    fn mark_query_saved(
        &mut self,
        tab_id: TabId,
        title: String,
        origin: QueryOrigin,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(
            AppCommand::MarkQuerySaved {
                tab_id,
                title,
                origin,
            },
            cx,
        );
    }
}

fn saved_query_tab_id(state: &AppState, query_id: u64) -> Option<TabId> {
    state.tabs.iter().find_map(|tab| match &tab.kind {
        TabKind::QueryEditor(editor)
            if editor.origin == Some(QueryOrigin::Connection { query_id }) =>
        {
            Some(tab.id)
        }
        _ => None,
    })
}

fn sql_path_with_extension(mut path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        path.set_extension("sql");
    }
    path
}

fn local_query_title(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("query.sql")
        .to_string()
}

fn default_sql_file_name(text: &str) -> String {
    format!("{}.sql", default_saved_query_name(text))
}

fn default_saved_query_name(text: &str) -> String {
    let name = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim())
        .unwrap_or("query");
    let name = safe_data_export_filename_segment(name);
    name.chars().take(48).collect()
}

async fn save_query_file(path: PathBuf, text: String) -> io::Result<PathBuf> {
    fs::write(&path, text).map(|_| path)
}
