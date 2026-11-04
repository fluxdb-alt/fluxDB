impl AppController {
    fn dispatch_ai_table_page(&mut self, command: AppCommand) -> AppEvent {
        match command {
            AppCommand::BrowseAiTableObjects {
                connection_id,
                parent,
            } => self.browse_ai_table_objects(connection_id, parent),
            AppCommand::FinishAiTableObjects {
                connection_id,
                parent,
                objects,
            } => {
                let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|c| c.config.id == connection_id)
                else {
                    return self.fail(editor_error("page_query_unavailable"));
                };
                if objects
                    .iter()
                    .any(|object| object.path.connection_id != connection_id)
                {
                    return self.fail(editor_error("page_query_invalid"));
                }
                if let Some(parent) = parent.as_ref() {
                    replace_loaded_children(&mut connection.objects, parent, objects.clone());
                } else {
                    replace_connection_level0(connection, objects.clone());
                }
                connection.connected = true;
                connection.expanded = true;
                AppEvent::ObjectsLoaded(parent, objects)
            }
            AppCommand::LoadAiTableColumns(object) => match self.ai_table_columns(&object) {
                Ok(columns) => AppEvent::AiTableColumnsLoaded(object, columns),
                Err(error) => self.fail(error),
            },
            AppCommand::RememberAiTableObject(object) => {
                let allowed = self.state.connections.iter().any(|connection| {
                    connection.config.id == object.connection_id
                        && matches!(
                            connection.config.kind,
                            DatabaseKind::Sqlite
                                | DatabaseKind::MySql
                                | DatabaseKind::TiDb
                                | DatabaseKind::MariaDb
                                | DatabaseKind::Postgres
                                | DatabaseKind::Dameng
                        )
                });
                if !allowed || !matches!(object.kind, ObjectKind::Table | ObjectKind::View) {
                    return self.fail(editor_error("page_query_unavailable"));
                }
                let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == object.connection_id)
                else {
                    return self.fail(editor_error("page_query_unavailable"));
                };
                if !connection.objects.iter().any(|item| item.path == object) {
                    connection.objects.push(ObjectSummary {
                        path: object,
                        rows: None,
                        modified_at: None,
                        comment: None,
                        stable: None,
                    });
                }
                connection.connected = true;
                AppEvent::ObjectsLoaded(None, connection.objects.clone())
            }
            AppCommand::BeginAiTableQuery { tab_id, limit } => {
                let Some(tab) = self.find_tab_mut(tab_id) else {
                    return self.fail(editor_error("page_query_unavailable"));
                };
                let TabKind::DataEditor(editor) = &mut tab.kind else {
                    return self.fail(editor_error("page_query_unavailable"));
                };
                if editor.loading
                    || editor.submitting
                    || editor.changes.as_ref().is_some_and(|c| !c.is_empty())
                {
                    return self.fail(editor_error("page_query_conflict"));
                }
                editor.pagination = Pagination::new(0, limit);
                let generation = editor.begin_load();
                AppEvent::DataPageLoadStarted { tab_id, generation }
            }
            AppCommand::LoadAiTablePage {
                tab_id,
                generation,
                object,
                query,
            } => {
                let available = self.find_tab(tab_id).is_some_and(|tab| match &tab.kind {
                    TabKind::DataEditor(editor) => {
                        editor.object == object
                            && editor.accepts_load(generation)
                            && !editor.submitting
                            && !editor.changes.as_ref().is_some_and(|c| !c.is_empty())
                    }
                    _ => false,
                });
                if !available || !query.execute {
                    return self.fail(editor_error("page_query_conflict"));
                }
                match self.load_ai_table_page(&object, &query) {
                    Ok(page) => AppEvent::DataLoaded(tab_id, page),
                    Err(error) => self.fail(error),
                }
            }
            _ => self.fail(editor_error("page_query_invalid")),
        }
    }
}
