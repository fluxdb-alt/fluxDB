impl AppController {
    /// 分发建表/设计表/表操作命令（T01 从巨型 dispatch match 按域提取）。
    fn dispatch_table_command(&mut self, command: AppCommand) -> AppEvent {
        match command {
            AppCommand::OpenCreateTable {
                connection_id,
                database,
                schema,
            } => {
                let Some(config) = self.connection_config(connection_id) else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let database_kind = config.kind;
                // 显式 schema 优先；缺省时用档案默认 schema，保证 PG 建表落在用户所见 schema（§9.2）。
                let create_schema = schema
                    .filter(|schema| !schema.trim().is_empty())
                    .or_else(|| {
                        config
                            .postgres_profile
                            .as_ref()
                            .map(|profile| profile.scope.default_schema.clone())
                            .or_else(|| {
                                config
                                    .dameng_profile
                                    .as_ref()
                                    .map(|profile| profile.basic.schema.clone())
                            })
                            .or_else(|| {
                                config
                                    .sqlserver_profile
                                    .as_ref()
                                    .map(|profile| profile.basic.schema.clone())
                            })
                            .filter(|schema| !schema.trim().is_empty())
                    })
                    .or_else(|| {
                        (database_kind == DatabaseKind::SqlServer).then(|| "dbo".to_string())
                    })
                    .unwrap_or_default();
                let tab_id = self.next_tab_id();
                let mut create = CreateTableState::new(connection_id, database, database_kind);
                create.schema = create_schema;
                self.push_tab(TabState {
                    id: tab_id,
                    title: create.tab_title(),
                    kind: TabKind::CreateTable(create),
                    dirty: false,
                });
                AppEvent::TabOpened(tab_id)
            }
            AppCommand::OpenDesignTable(object) => {
                match self.prepare_design_table(&object) {
                    Ok(create) => self.install_design_table(create),
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::LoadDesignTable(object) => match self.prepare_design_table(&object) {
                Ok(create) => AppEvent::DesignTableLoaded(Ok(create)),
                Err(error) => AppEvent::DesignTableLoaded(Err(UserFacingError::from(error))),
            },
            AppCommand::LoadDesignTableBase(object) => {
                match self.prepare_design_table_base(&object) {
                    Ok(create) => AppEvent::DesignTableLoaded(Ok(create)),
                    Err(error) => AppEvent::DesignTableLoaded(Err(UserFacingError::from(error))),
                }
            }
            AppCommand::LoadDesignTableDetails(object) => {
                match self.load_design_table_details(&object) {
                    Ok(metadata) => AppEvent::DesignTableDetailsLoaded(metadata),
                    Err(error) => AppEvent::Failed(UserFacingError::from(error)),
                }
            }
            AppCommand::FinishOpenDesignTable { result } => match result {
                Ok(create) => self.install_design_table(create),
                Err(error) => {
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            AppCommand::FinishDesignTableDetails { tab_id, metadata } => {
                let Some(tab) = self.find_tab_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "设计表标签页不存在"));
                };
                let TabKind::CreateTable(create) = &mut tab.kind else {
                    return self.fail(Error::new(ErrorKind::Internal, "设计表标签页不存在"));
                };
                if let Err(message) = create.finish_design_details(metadata) {
                    return self.fail(Error::new(ErrorKind::Query, message));
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::RenameTable { object, new_name } => {
                let Some(config) = self.connection_config(object.connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let new_name = new_name.trim().to_string();
                let sql = match rename_table_sql_preview(
                    config.kind,
                    object.schema.as_deref(),
                    &object.name,
                    &new_name,
                ) {
                    Ok(sql) => sql,
                    Err(message) => return self.fail(Error::new(ErrorKind::Query, message)),
                };
                let request = QueryRequest {
                    connection_id: object.connection_id,
                    database: object.database.clone(),
                    session_id: None,
                    schema: object.schema.clone(),
                    text: sql,
                    mode: fluxdb_core::QueryMode::All,
                    options: QueryExecutionOptions {
                        continue_on_error: false,
                        split_statements: true,
                        ..QueryExecutionOptions::default()
                    },
                };
                let execution = match self.execute_query(&request) {
                    Ok(execution) => execution,
                    Err(error) => return self.fail(error),
                };
                if let Some(summary) = execution.summaries.iter().find(|summary| !summary.success) {
                    return self.fail(Error::new(ErrorKind::Query, summary.message.clone()));
                }

                let mut renamed = object.clone();
                renamed.name = new_name.clone();
                if let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == object.connection_id)
                {
                    for summary in &mut connection.objects {
                        if summary.path == object {
                            summary.path = renamed.clone();
                        }
                    }
                }
                for tab in &mut self.state.tabs {
                    if let TabKind::DataEditor(editor) = &mut tab.kind
                        && editor.object == object
                    {
                        editor.object = renamed.clone();
                        editor.page = None;
                        editor.original_page = None;
                        editor.changes = None;
                        editor.begin_load();
                        editor.table_info = TableInfoState::default();
                        tab.title = new_name.clone();
                        tab.dirty = false;
                    }
                }
                // 表操作成功后失效该 scope 的补全缓存：旧名不再被建议、新名能尽快出现。
                self.mark_table_action_completion_dirty(&object);
                self.state.last_error = None;
                AppEvent::TableRenamed { object, new_name }
            }
            AppCommand::CopyTable {
                object,
                new_name,
                copy_data,
            } => {
                let Some(config) = self.connection_config(object.connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let new_name = new_name.trim().to_string();
                // SQLite/达梦/SQL Server/Oracle 的复制表要在客户端重写 DDL（保留约束与索引），
                // 需要先读取源表定义。
                let source_ddl = if copy_table_requires_source_ddl(config.kind) {
                    match self.load_table_ddl(&object) {
                        Ok(ddl) => Some(ddl),
                        Err(error) => return self.fail(error),
                    }
                } else {
                    None
                };
                let sql = match copy_table_sql_preview_with_source_ddl(
                    config.kind,
                    object.schema.as_deref(),
                    &object.name,
                    &new_name,
                    copy_data,
                    source_ddl.as_deref(),
                ) {
                    Ok(sql) => sql,
                    Err(message) => return self.fail(Error::new(ErrorKind::Query, message)),
                };
                let (sql, split_statements) = if config.kind == DatabaseKind::SqlServer {
                    // DDL 后置的索引/外键失败时，不能留下只建了一半的复制表。
                    (
                        format!(
                            "BEGIN TRY\nBEGIN TRANSACTION;\n{sql}\nCOMMIT TRANSACTION;\nEND TRY\nBEGIN CATCH\nIF @@TRANCOUNT > 0 ROLLBACK TRANSACTION;\nTHROW;\nEND CATCH"
                        ),
                        false,
                    )
                } else {
                    (sql, true)
                };
                let request = QueryRequest {
                    connection_id: object.connection_id,
                    database: object.database.clone(),
                    session_id: None,
                    schema: object.schema.clone(),
                    text: sql,
                    mode: fluxdb_core::QueryMode::All,
                    options: QueryExecutionOptions {
                        continue_on_error: false,
                        split_statements,
                        ..QueryExecutionOptions::default()
                    },
                };
                let execution = match self.execute_query(&request) {
                    Ok(execution) => execution,
                    Err(error) => return self.fail(error),
                };
                if let Some(summary) = execution.summaries.iter().find(|summary| !summary.success) {
                    return self.fail(Error::new(ErrorKind::Query, summary.message.clone()));
                }

                let mut copied = object.clone();
                copied.name = new_name.clone();
                if let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == object.connection_id)
                    && !connection
                        .objects
                        .iter()
                        .any(|summary| summary.path == copied)
                {
                    connection.objects.push(ObjectSummary {
                        path: copied,
                        rows: None,
                        size_bytes: None,
                        modified_at: None,
                        comment: None,
                        stable: None,
                    });
                }
                // 新表的列/索引需要进入补全缓存：标记 scope 失效，后台刷新时重取表清单。
                self.mark_table_action_completion_dirty(&object);
                self.state.last_error = None;
                AppEvent::TableCopied { object, new_name }
            }
            AppCommand::DropTable {
                object,
                foreign_key_check,
            } => {
                let Some(config) = self.connection_config(object.connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let sql = match drop_table_sql_preview(
                    config.kind,
                    object.kind,
                    object.schema.as_deref(),
                    &object.name,
                    foreign_key_check,
                ) {
                    Ok(sql) => sql,
                    Err(message) => return self.fail(Error::new(ErrorKind::Query, message)),
                };
                let request = QueryRequest {
                    connection_id: object.connection_id,
                    database: object.database.clone(),
                    session_id: None,
                    schema: object.schema.clone(),
                    text: sql,
                    mode: fluxdb_core::QueryMode::All,
                    options: QueryExecutionOptions {
                        continue_on_error: false,
                        split_statements: true,
                        ..QueryExecutionOptions::default()
                    },
                };
                let execution = match self.execute_query(&request) {
                    Ok(execution) => execution,
                    Err(error) => return self.fail(error),
                };
                if let Some(summary) = execution.summaries.iter().find(|summary| !summary.success) {
                    return self.fail(Error::new(ErrorKind::Query, summary.message.clone()));
                }

                if let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == object.connection_id)
                {
                    connection.objects.retain(|summary| summary.path != object);
                }
                self.state.tabs.retain(|tab| match &tab.kind {
                    TabKind::DataEditor(editor) => editor.object != object,
                    TabKind::CreateTable(create) => match &create.mode {
                        CreateTableMode::Design { object: design_object, .. } => {
                            design_object != &object
                        }
                        CreateTableMode::Create => true,
                    },
                    _ => true,
                });
                if self
                    .state
                    .active_tab
                    .is_some_and(|tab_id| !self.state.tabs.iter().any(|tab| tab.id == tab_id))
                {
                    self.state.active_tab = self.state.tabs.last().map(|tab| tab.id);
                }
                // 删除后必须让索引知道：表名先按 scope 级失效，后台刷新重取表清单时移除。
                self.mark_table_action_completion_dirty(&object);
                self.state.last_error = None;
                AppEvent::TableDropped(object)
            }
            AppCommand::TruncateTable {
                object,
                foreign_key_check,
                restart_identity,
            } => {
                let Some(config) = self.connection_config(object.connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let sql = match truncate_table_sql_preview(
                    config.kind,
                    object.schema.as_deref(),
                    &object.name,
                    restart_identity,
                    foreign_key_check,
                ) {
                    Ok(sql) => sql,
                    Err(message) => return self.fail(Error::new(ErrorKind::Query, message)),
                    };
                let request = QueryRequest {
                    connection_id: object.connection_id,
                    database: object.database.clone(),
                    session_id: None,
                    schema: object.schema.clone(),
                    text: sql,
                    mode: fluxdb_core::QueryMode::All,
                    options: QueryExecutionOptions {
                        continue_on_error: false,
                        split_statements: true,
                        ..QueryExecutionOptions::default()
                    },
                };
                let execution = match self.execute_query(&request) {
                    Ok(execution) => execution,
                    Err(error) => return self.fail(error),
                };
                if let Some(summary) = execution.summaries.iter().find(|summary| !summary.success) {
                    return self.fail(Error::new(ErrorKind::Query, summary.message.clone()));
                }

                self.state.last_error = None;
                AppEvent::TableTruncated(object)
            }
            AppCommand::StartCreateTableApply(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    if let Some(message) = create.validation_error() {
                        return self.fail(Error::new(ErrorKind::Query, message));
                    }
                    create.applying = true;
                    create.apply_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::ApplyCreateTable(tab_id) => match self.apply_create_table(tab_id) {
                Ok(()) => AppEvent::CreateTableApplied(tab_id),
                Err(error) => AppEvent::Failed(UserFacingError::from(error)),
            },
            AppCommand::FinishCreateTableApply { tab_id, result } => match result {
                Ok(()) => {
                    let refresh_design = match self.find_tab_mut(tab_id) {
                        Some(tab) => match &mut tab.kind {
                            TabKind::CreateTable(create) => {
                                create.applying = false;
                                create.apply_error = None;
                                tab.dirty = false;
                                create.is_design()
                                    && matches!(
                                        create.database_kind,
                                        DatabaseKind::Postgres
                                            | DatabaseKind::Kingbase
                                            | DatabaseKind::Dameng
                                            | DatabaseKind::SqlServer
                                    )
                            }
                            _ => false,
                        },
                        None => return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在")),
                    };
                    // PG/达梦设计表保存成功后按落库结构重建基线：否则 original_ddl 停留在打开时快照，
                    // 下次保存会把「自己刚保存的改动」误判为外部变化而拒绝（§9.2）。
                    if refresh_design
                        && let Err(error) = self.refresh_design_table_after_apply(tab_id)
                    {
                        tracing::warn!(
                            target: "gdb_create_table",
                            ?tab_id,
                            error = %error,
                            "保存后刷新设计基线失败，保留当前编辑状态"
                        );
                    }
                    AppEvent::CreateTableApplied(tab_id)
                }
                Err(error) => {
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::CreateTable(create) = &mut tab.kind
                    {
                        create.applying = false;
                        create.apply_error = Some(error.clone());
                    }
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            AppCommand::SetCreateTableField {
                tab_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_field(field, value);
                    tab.title = create.tab_title();
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableOptionField {
                tab_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_option_field(field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::ToggleCreateTablePartitionEnabled(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.toggle_partition_enabled();
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTablePartitionField {
                tab_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_partition_field(field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SelectCreateTableTab { tab_id, create_tab } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                if create_tab == CreateTableTab::Ddl && !create.is_design() {
                    return AppEvent::TabActivated(tab_id);
                }
                create.active_tab = create_tab;
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SelectCreateTableColumn { tab_id, column_id } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                create.select_column(column_id);
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::AddCreateTableColumn(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_column();
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableColumnUp { tab_id, column_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_column_up(column_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableColumnDown { tab_id, column_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_column_down(column_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableColumn { tab_id, column_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_column(column_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableColumnField {
                tab_id,
                column_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_column_field(column_id, field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::AddCreateTableColumnEnumValue { tab_id, column_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_column_enum_value(column_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableColumnEnumValue {
                tab_id,
                column_id,
                index,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_column_enum_value(column_id, index, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableColumnEnumValue {
                tab_id,
                column_id,
                index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_column_enum_value(column_id, index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableColumnEnumValueUp {
                tab_id,
                column_id,
                index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_column_enum_value_up(column_id, index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableColumnEnumValueDown {
                tab_id,
                column_id,
                index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_column_enum_value_down(column_id, index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::ToggleCreateTableColumnFlag {
                tab_id,
                column_id,
                flag,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.toggle_column_flag(column_id, flag);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SelectCreateTableIndex { tab_id, index_id } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                create.select_index(index_id);
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::AddCreateTableIndex(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_index();
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableIndexUp { tab_id, index_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_index_up(index_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableIndexDown { tab_id, index_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_index_down(index_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableIndex { tab_id, index_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_index(index_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableIndexField {
                tab_id,
                index_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_index_field(index_id, field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::AddCreateTableIndexColumn { tab_id, index_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_index_column(index_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableIndexColumnUp {
                tab_id,
                index_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_index_column_up(index_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableIndexColumnDown {
                tab_id,
                index_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_index_column_down(index_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableIndexColumn {
                tab_id,
                index_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_index_column(index_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableIndexColumnField {
                tab_id,
                index_id,
                column_index,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_index_column_field(index_id, column_index, field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SelectCreateTableCheck { tab_id, check_id } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                create.select_check(check_id);
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::AddCreateTableCheck(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_check();
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableCheckUp { tab_id, check_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_check_up(check_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableCheckDown { tab_id, check_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_check_down(check_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableCheck { tab_id, check_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_check(check_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableCheckField {
                tab_id,
                check_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_check_field(check_id, field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::ToggleCreateTableCheckNotEnforced { tab_id, check_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.toggle_check_not_enforced(check_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SelectCreateTableForeignKey {
                tab_id,
                foreign_key_id,
            } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                create.select_foreign_key(foreign_key_id);
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::AddCreateTableForeignKey(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_foreign_key();
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableForeignKeyUp {
                tab_id,
                foreign_key_id,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_foreign_key_up(foreign_key_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableForeignKeyDown {
                tab_id,
                foreign_key_id,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_foreign_key_down(foreign_key_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableForeignKey {
                tab_id,
                foreign_key_id,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_foreign_key(foreign_key_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableForeignKeyField {
                tab_id,
                foreign_key_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_foreign_key_field(foreign_key_id, field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::StartCreateTableReferenceColumnsLoad {
                tab_id,
                foreign_key_id,
            } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                create.start_foreign_key_reference_columns_load(foreign_key_id);
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::LoadCreateTableReferenceColumns {
                tab_id,
                foreign_key_id,
            } => match self.load_create_table_reference_columns(tab_id, foreign_key_id) {
                Ok(columns) => AppEvent::CreateTableReferenceColumnsLoaded {
                    tab_id,
                    foreign_key_id,
                    columns,
                },
                Err(error) => AppEvent::Failed(UserFacingError::from(error)),
            },
            AppCommand::FinishCreateTableReferenceColumnsLoad {
                tab_id,
                foreign_key_id,
                result,
            } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                create.finish_foreign_key_reference_columns_load(foreign_key_id, result.clone());
                match result {
                    Ok(columns) => AppEvent::CreateTableReferenceColumnsLoaded {
                        tab_id,
                        foreign_key_id,
                        columns,
                    },
                    Err(error) => {
                        self.state.last_error = Some(error.clone());
                        AppEvent::Failed(error)
                    }
                }
            }
            AppCommand::AddCreateTableForeignKeyColumn {
                tab_id,
                foreign_key_id,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_foreign_key_column(foreign_key_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::AddCreateTableForeignKeyReferencedColumn {
                tab_id,
                foreign_key_id,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_foreign_key_referenced_column(foreign_key_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableForeignKeyReferencedColumnUp {
                tab_id,
                foreign_key_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_foreign_key_referenced_column_up(foreign_key_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableForeignKeyReferencedColumnDown {
                tab_id,
                foreign_key_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_foreign_key_referenced_column_down(foreign_key_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableForeignKeyReferencedColumn {
                tab_id,
                foreign_key_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_foreign_key_referenced_column(foreign_key_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableForeignKeyReferencedColumn {
                tab_id,
                foreign_key_id,
                column_index,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_foreign_key_referenced_column(foreign_key_id, column_index, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableForeignKeyColumnUp {
                tab_id,
                foreign_key_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_foreign_key_column_up(foreign_key_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableForeignKeyColumnDown {
                tab_id,
                foreign_key_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_foreign_key_column_down(foreign_key_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableForeignKeyColumn {
                tab_id,
                foreign_key_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_foreign_key_column(foreign_key_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableForeignKeyColumn {
                tab_id,
                foreign_key_id,
                column_index,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_foreign_key_column(foreign_key_id, column_index, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SelectCreateTableTrigger { tab_id, trigger_id } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                create.select_trigger(trigger_id);
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::AddCreateTableTrigger(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_trigger();
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableTriggerUp { tab_id, trigger_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_trigger_up(trigger_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableTriggerDown { tab_id, trigger_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_trigger_down(trigger_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableTrigger { tab_id, trigger_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_trigger(trigger_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableTriggerField {
                tab_id,
                trigger_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_trigger_field(trigger_id, field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableTriggerEvent {
                tab_id,
                trigger_id,
                event,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_trigger_event(trigger_id, event);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }

            _ => {
                self.fail(Error::new(
                    ErrorKind::Internal,
                    "非表操作命令不应分发至表操作处理器",
                ))
            }
        }
    }

    /// 读取设计表所需元数据并构造尚未安装的标签页状态。
    ///
    /// 该方法不修改控制器状态，可由后台控制器快照调用；尤其是同步 DM 驱动和远端
    /// MySQL/PG 连接不能放到 GPUI 主线程执行。
    fn prepare_design_table(&self, object: &ObjectPath) -> fluxdb_core::Result<CreateTableState> {
        let Some(config) = self.connection_config(object.connection_id) else {
            return Err(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        let metadata = load_table_design_for_connection(config, object)?;
        Ok(CreateTableState::design(
            object.clone(),
            config.kind,
            metadata.columns,
            metadata.indexes,
            metadata.foreign_keys,
            metadata.triggers,
            Some(metadata.ddl),
        ))
    }

    fn prepare_design_table_base(
        &self,
        object: &ObjectPath,
    ) -> fluxdb_core::Result<CreateTableState> {
        let Some(config) = self.connection_config(object.connection_id) else {
            return Err(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        let metadata = load_table_design_base_for_connection(config, object)?;
        let details_loading = !metadata.details_loaded;
        let mut create = CreateTableState::design(
            object.clone(),
            config.kind,
            metadata.columns,
            metadata.indexes,
            metadata.foreign_keys,
            metadata.triggers,
            Some(metadata.ddl),
        );
        create.details_loading = details_loading;
        Ok(create)
    }

    fn load_design_table_details(
        &self,
        object: &ObjectPath,
    ) -> fluxdb_core::Result<fluxdb_core::TableDesignMetadata> {
        let Some(config) = self.connection_config(object.connection_id) else {
            return Err(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        load_table_design_for_connection(config, object)
    }

    /// 把已加载的设计表状态安装为标签页。调用发生在主线程且不做 IO。
    fn install_design_table(&mut self, create: CreateTableState) -> AppEvent {
        let tab_id = self.next_tab_id();
        self.push_tab(TabState {
            id: tab_id,
            title: create.tab_title(),
            kind: TabKind::CreateTable(create),
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
    }

    /// PG/达梦设计表保存成功后，用落库后的最新结构重建该 tab 的设计基线。
    ///
    /// 复用打开设计器的全部元数据加载路径（列/索引/外键/触发器/DDL），把 `original_ddl`
    /// 与 `original` 快照推进到保存后的状态；否则下次保存会把本次已落库的改动与旧基线比较，
    /// 误判为「表结构已在外部变化」而拒绝（§9.2 外部 DDL 保护）。
    fn refresh_design_table_after_apply(&mut self, tab_id: TabId) -> fluxdb_core::Result<()> {
        let object = {
            let Some(tab) = self.find_tab(tab_id) else {
                return Err(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
            };
            let TabKind::CreateTable(create) = &tab.kind else {
                return Ok(());
            };
            let CreateTableMode::Design { object, .. } = &create.mode else {
                return Ok(());
            };
            object.clone()
        };
        let fresh = self.prepare_design_table(&object)?;
        if let Some(tab) = self.find_tab_mut(tab_id)
            && let TabKind::CreateTable(create) = &mut tab.kind
        {
            *create = fresh;
            tab.dirty = false;
        }
        Ok(())
    }
}
