const CREATE_TABLE_EDITOR_MIN_HEIGHT: f32 = 260.;
const CREATE_TABLE_INPUT_HEIGHT: f32 = 26.;
const CREATE_TABLE_OPERATIONS_WIDTH: f32 = 132.;
const CREATE_TABLE_FIELDS_TABLE_WIDTH: f32 =
    46. + 190. + 150. + 100. + 86. + 68. + 160. + 180. + CREATE_TABLE_OPERATIONS_WIDTH;
const CREATE_TABLE_COMMENT_EDITOR_DEFAULT_WIDTH: f32 = 420.;
const CREATE_TABLE_COMMENT_EDITOR_DEFAULT_HEIGHT: f32 = 220.;
const CREATE_TABLE_COMMENT_EDITOR_MIN_WIDTH: f32 = 320.;
const CREATE_TABLE_COMMENT_EDITOR_MIN_HEIGHT: f32 = 170.;
/// 选项始终保存 SQL 值；支持继承的字段以空串表示继承，其他字段保留空白语义。
#[derive(Clone)]
struct CreateTableSelectOption {
    value: String,
    inherit_default: bool,
}

#[derive(Clone)]
struct CreateTableTypeOption {
    value: String,
    custom_entry: bool,
}

impl SelectItem for CreateTableTypeOption {
    type Value = String;

    fn title(&self) -> SharedString {
        if self.custom_entry {
            tr("app.create_table.column.custom_type_action")
        } else {
            self.value.clone().into()
        }
    }

    fn value(&self) -> &String {
        &self.value
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let debug_id = if self.custom_entry {
            "create-table-type-option:custom".to_string()
        } else {
            format!("create-table-type-option:{}", self.value)
        };
        div()
            .debug_selector(move || debug_id.clone())
            .child(self.title())
    }
}

impl SelectItem for CreateTableSelectOption {
    type Value = String;

    fn title(&self) -> SharedString {
        if self.value.is_empty() && self.inherit_default {
            tr("app.create_table.inherit_database_default")
        } else {
            self.value.clone().into()
        }
    }

    fn value(&self) -> &String {
        &self.value
    }

    fn display_title(&self) -> Option<gpui::AnyElement> {
        let label = self.title();
        Some(
            div()
                .debug_selector({
                    let label = label.clone();
                    move || format!("create-table-selected:{label}")
                })
                .child(label)
                .into_any_element(),
        )
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let value = self.value.clone();
        div()
            .debug_selector(move || format!("create-table-option:{value}"))
            .child(self.title())
    }
}
const CREATE_TABLE_INDEX_FIELDS_POPOVER_WIDTH: f32 = 388.;
const CREATE_TABLE_INDEX_FIELD_NAME_WIDTH: f32 = 178.;
const CREATE_TABLE_INDEX_FIELD_SUB_PART_WIDTH: f32 = 82.;
const CREATE_TABLE_INDEX_FIELD_SORT_ORDER_WIDTH: f32 = 112.;
const CREATE_TABLE_INDEX_FIELD_DROPDOWN_EXTRA_HEIGHT: f32 = 156.;

#[derive(Clone)]
struct CreateTableFieldsTableRow {
    index: usize,
    column: CreateTableColumn,
    name_input: Entity<InputState>,
    type_select: Entity<SelectState<SearchableVec<CreateTableTypeOption>>>,
    type_input: Entity<InputState>,
    length_input: Entity<InputState>,
    default_input: Entity<InputState>,
    comment_input: Entity<InputState>,
    comment_editor_input: Entity<InputState>,
}


include!("create_table/indexes.rs");
include!("create_table/foreign_keys.rs");
include!("create_table/checks.rs");
include!("create_table/options.rs");
include!("create_table/triggers.rs");

impl NavicatMain {
    fn create_table_input(
        &mut self,
        key: CreateTableInputKey,
        placeholder: impl Into<SharedString>,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let placeholder = placeholder.into();
        if !self.create_table_inputs.contains_key(&key) {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder.clone())
                    .default_value(value.to_string())
            });
            let subscription = cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
                if let CreateTableInputKey::ColumnCustomType(tab_id, column_id) = key {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        this.finish_create_table_custom_type_edit(
                            tab_id,
                            column_id,
                            input.read(cx).value().to_string(),
                            cx,
                        );
                    }
                    return;
                }
                if matches!(event, InputEvent::Change) {
                    this.update_create_table_input(key, input.read(cx).value().to_string(), cx);
                }
            });
            self.create_table_inputs.insert(key, input);
            self._create_table_input_subscriptions
                .insert(key, subscription);
        }

        let input = self.create_table_inputs.get(&key).cloned().unwrap();
        input.update(cx, |input, cx| {
            input.set_placeholder(placeholder.clone(), window, cx);
        });
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        let should_clear_stale_disabled_value = value.is_empty() && placeholder.is_empty();
        if (!focused || should_clear_stale_disabled_value) && input.read(cx).value().as_ref() != value
        {
            input.update(cx, |input, cx| {
                input.set_value(value.to_string(), window, cx);
            });
        }
        input
    }

    fn update_create_table_input(
        &mut self,
        key: CreateTableInputKey,
        value: String,
        cx: &mut Context<Self>,
    ) {
        match key {
            CreateTableInputKey::TableName(tab_id) => {
                self.dispatch(
                    AppCommand::SetCreateTableField {
                        tab_id,
                        field: CreateTableField::TableName,
                        value,
                    },
                    cx,
                );
            }
            CreateTableInputKey::TableComment(tab_id) => {
                self.dispatch(
                    AppCommand::SetCreateTableField {
                        tab_id,
                        field: CreateTableField::Comment,
                        value,
                    },
                    cx,
                );
            }
            CreateTableInputKey::ColumnName(tab_id, column_id) => {
                self.set_create_table_column_input(
                    tab_id,
                    column_id,
                    CreateTableColumnField::Name,
                    value,
                    cx,
                );
            }
            CreateTableInputKey::ColumnLength(tab_id, column_id) => {
                self.set_create_table_column_input(
                    tab_id,
                    column_id,
                    CreateTableColumnField::Length,
                    value,
                    cx,
                );
            }
            // 自定义类型在 Enter/Blur 时整体提交，输入过程中只保留在输入框草稿里。
            CreateTableInputKey::ColumnCustomType(_, _) => {}
            CreateTableInputKey::ColumnEnumValue(tab_id, column_id, index) => {
                self.dispatch(
                    AppCommand::SetCreateTableColumnEnumValue {
                        tab_id,
                        column_id,
                        index,
                        value,
                    },
                    cx,
                );
            }
            CreateTableInputKey::ColumnScale(tab_id, column_id) => {
                self.set_create_table_column_input(
                    tab_id,
                    column_id,
                    CreateTableColumnField::Scale,
                    value,
                    cx,
                );
            }
            CreateTableInputKey::ColumnDefault(tab_id, column_id) => {
                self.set_create_table_column_input(
                    tab_id,
                    column_id,
                    CreateTableColumnField::DefaultValue,
                    value,
                    cx,
                );
            }
            CreateTableInputKey::ColumnComment(tab_id, column_id)
            | CreateTableInputKey::ColumnCommentEditor(tab_id, column_id) => {
                self.set_create_table_column_input(
                    tab_id,
                    column_id,
                    CreateTableColumnField::Comment,
                    value,
                    cx,
                );
            }
            CreateTableInputKey::ColumnKeyLength(tab_id, column_id) => {
                self.set_create_table_column_input(
                    tab_id,
                    column_id,
                    CreateTableColumnField::KeyLength,
                    value,
                    cx,
                );
            }
            CreateTableInputKey::IndexName(tab_id, index_id) => {
                self.dispatch(
                    AppCommand::SetCreateTableIndexField {
                        tab_id,
                        index_id,
                        field: CreateTableIndexField::Name,
                        value,
                    },
                    cx,
                );
            }
            CreateTableInputKey::IndexComment(tab_id, index_id) => {
                self.dispatch(
                    AppCommand::SetCreateTableIndexField {
                        tab_id,
                        index_id,
                        field: CreateTableIndexField::Comment,
                        value,
                    },
                    cx,
                );
            }
            CreateTableInputKey::IndexColumnSubPart(tab_id, index_id, column_index) => {
                self.dispatch(
                    AppCommand::SetCreateTableIndexColumnField {
                        tab_id,
                        index_id,
                        column_index,
                        field: CreateTableIndexColumnField::SubPart,
                        value,
                    },
                    cx,
                );
            }
            CreateTableInputKey::ForeignKeyName(tab_id, foreign_key_id) => {
                self.dispatch(
                    AppCommand::SetCreateTableForeignKeyField {
                        tab_id,
                        foreign_key_id,
                        field: CreateTableForeignKeyField::Name,
                        value,
                    },
                    cx,
                );
            }
            CreateTableInputKey::CheckName(tab_id, check_id) => {
                self.dispatch(
                    AppCommand::SetCreateTableCheckField {
                        tab_id,
                        check_id,
                        field: CreateTableCheckField::Name,
                        value,
                    },
                    cx,
                );
            }
            CreateTableInputKey::CheckExpression(tab_id, check_id)
            | CreateTableInputKey::CheckExpressionEditor(tab_id, check_id) => {
                self.dispatch(
                    AppCommand::SetCreateTableCheckField {
                        tab_id,
                        check_id,
                        field: CreateTableCheckField::Expression,
                        value,
                    },
                    cx,
                );
            }
            CreateTableInputKey::TriggerName(tab_id, trigger_id) => {
                self.dispatch(
                    AppCommand::SetCreateTableTriggerField {
                        tab_id,
                        trigger_id,
                        field: CreateTableTriggerField::Name,
                        value,
                    },
                    cx,
                );
            }
            CreateTableInputKey::TriggerBody(tab_id, trigger_id) => {
                self.dispatch(
                    AppCommand::SetCreateTableTriggerField {
                        tab_id,
                        trigger_id,
                        field: CreateTableTriggerField::Body,
                        value,
                    },
                    cx,
                );
            }
            CreateTableInputKey::TableTablespace(tab_id) => {
                self.set_create_table_option_input(
                    tab_id,
                    CreateTableOptionField::Tablespace,
                    value,
                    cx,
                );
            }
            CreateTableInputKey::TableAvgRowLength(tab_id) => {
                self.set_create_table_option_input(
                    tab_id,
                    CreateTableOptionField::AvgRowLength,
                    value,
                    cx,
                );
            }
            CreateTableInputKey::TableMaxRows(tab_id) => {
                self.set_create_table_option_input(tab_id, CreateTableOptionField::MaxRows, value, cx);
            }
            CreateTableInputKey::TableMinRows(tab_id) => {
                self.set_create_table_option_input(tab_id, CreateTableOptionField::MinRows, value, cx);
            }
            CreateTableInputKey::TableKeyBlockSize(tab_id) => {
                self.set_create_table_option_input(
                    tab_id,
                    CreateTableOptionField::KeyBlockSize,
                    value,
                    cx,
                );
            }
            CreateTableInputKey::TablePartitionExpression(tab_id) => {
                self.dispatch(
                    AppCommand::SetCreateTablePartitionField {
                        tab_id,
                        field: CreateTablePartitionField::Expression,
                        value,
                    },
                    cx,
                );
            }
            CreateTableInputKey::TablePartitionSql(tab_id) => {
                self.dispatch(
                    AppCommand::SetCreateTablePartitionField {
                        tab_id,
                        field: CreateTablePartitionField::Sql,
                        value,
                    },
                    cx,
                );
            }
        }
    }

    fn set_create_table_column_input(
        &mut self,
        tab_id: TabId,
        column_id: u64,
        field: CreateTableColumnField,
        value: String,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(
            AppCommand::SetCreateTableColumnField {
                tab_id,
                column_id,
                field,
                value,
            },
            cx,
        );
    }

    fn finish_create_table_custom_type_edit(
        &mut self,
        tab_id: TabId,
        column_id: u64,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if self.create_table_custom_type_editing != Some((tab_id, column_id)) {
            return;
        }
        let value = value.trim().to_string();
        if !value.is_empty() {
            self.set_create_table_column_input(
                tab_id,
                column_id,
                CreateTableColumnField::CustomType,
                value,
                cx,
            );
        }
        self.create_table_custom_type_editing = None;
        cx.notify();
    }

    fn create_table_enum_scroll(&mut self, tab_id: TabId, column_id: u64) -> ScrollHandle {
        self.create_table_enum_scrolls
            .entry((tab_id, column_id))
            .or_default()
            .clone()
    }

    fn set_create_table_option_input(
        &mut self,
        tab_id: TabId,
        field: CreateTableOptionField,
        value: String,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(
            AppCommand::SetCreateTableOptionField {
                tab_id,
                field,
                value,
            },
            cx,
        );
    }

    fn start_create_table_apply(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self._create_table_apply_tasks.contains_key(&tab_id) {
            self.show_message(
                tr("app.create_table.saving").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }

        let Some(create) = self
            .controller
            .state()
            .tabs
            .iter()
            .find_map(|tab| match &tab.kind {
                TabKind::CreateTable(create) if tab.id == tab_id => Some(create.clone()),
                _ => None,
            })
        else {
            self.show_message(
                tr("app.create_table.error.tab_missing").as_ref(),
                AppMessageKind::Error,
                cx,
            );
            return;
        };
        if let Some(message) = create.validation_error() {
            self.show_message(message, AppMessageKind::Warning, cx);
            return;
        }

        let event = self.dispatch(AppCommand::StartCreateTableApply(tab_id), cx);
        if let AppEvent::Failed(error) = event {
            self.show_message(error.message, AppMessageKind::Error, cx);
            return;
        }

        let mut controller = self.controller.clone();
        let connection_id = create.connection_id;
        let database = create.database.clone();
        let is_design = create.is_design();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::ApplyCreateTable(tab_id)) {
                        AppEvent::CreateTableApplied(_) => Ok(()),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: tr("app.common.save_failed").to_string(),
                            message: tr("app.create_table.error.save_no_result").to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._create_table_apply_tasks.remove(&tab_id);
                    let success = result.is_ok();
                    this.dispatch(
                        AppCommand::FinishCreateTableApply {
                            tab_id,
                            result: result.clone(),
                        },
                        cx,
                    );
                    match result {
                        Ok(()) => {
                            let message = if is_design {
                                tr("app.create_table.toast.design_saved")
                            } else {
                                tr("app.create_table.toast.created")
                            };
                            this.show_message(
                                message.as_ref(),
                                AppMessageKind::Success,
                                cx,
                            );
                            if let Some(database) = database {
                                // PG 按 schema 懒加载：建表成功后只重载新表所在 schema，避免
                                // 把整库（DB 级）表/视图行当 schema_scope=None 清空（见 replace_loaded_children）。
                                if create.database_kind == DatabaseKind::Postgres {
                                    let schema = if create.schema.trim().is_empty() {
                                        "public"
                                    } else {
                                        &create.schema
                                    };
                                    let key = schema_tree_key(connection_id, &database, schema);
                                    this.load_database_children(
                                        ObjectPath {
                                            connection_id,
                                            database: Some(database.clone()),
                                            schema: Some(schema.to_string()),
                                            name: schema.to_string(),
                                            kind: ObjectKind::Schema,
                                        },
                                        key,
                                        cx,
                                    );
                                } else {
                                    let key = database_tree_key(connection_id, &database);
                                    this.load_database_children(
                                        ObjectPath {
                                            connection_id,
                                            database: Some(database.clone()),
                                            schema: None,
                                            name: database,
                                            kind: ObjectKind::Database,
                                        },
                                        key,
                                        cx,
                                    );
                                }
                            }
                        }
                        Err(error) => this.show_message(error.message, AppMessageKind::Error, cx),
                    }
                    if success {
                        cx.notify();
                    }
                });
            });
        });
        self._create_table_apply_tasks.insert(tab_id, task);
        cx.notify();
    }

    fn start_create_table_reference_columns_load(
        &mut self,
        tab_id: TabId,
        foreign_key_id: u64,
        cx: &mut Context<Self>,
    ) {
        let task_key = (tab_id, foreign_key_id);
        if self
            ._create_table_reference_columns_tasks
            .contains_key(&task_key)
        {
            return;
        }
        let Some(should_load) = self
            .controller
            .state()
            .tabs
            .iter()
            .find_map(|tab| match &tab.kind {
                TabKind::CreateTable(create) if tab.id == tab_id => {
                    create
                        .foreign_keys
                        .iter()
                        .find(|foreign_key| foreign_key.id == foreign_key_id)
                        .map(|foreign_key| {
                            !foreign_key.referenced_table.trim().is_empty()
                                && matches!(
                                    foreign_key.referenced_column_options,
                                    LoadState::NotLoaded | LoadState::Failed(_)
                                )
                        })
                }
                _ => None,
            })
        else {
            return;
        };
        if !should_load {
            return;
        }

        self.dispatch(
            AppCommand::StartCreateTableReferenceColumnsLoad {
                tab_id,
                foreign_key_id,
            },
            cx,
        );
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadCreateTableReferenceColumns {
                        tab_id,
                        foreign_key_id,
                    }) {
                        AppEvent::CreateTableReferenceColumnsLoaded { columns, .. } => Ok(columns),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: tr("app.create_table.error.load_failed").to_string(),
                            message: tr("app.create_table.error.load_field_no_result")
                                .to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._create_table_reference_columns_tasks
                        .remove(&task_key);
                    let event = this.dispatch(
                        AppCommand::FinishCreateTableReferenceColumnsLoad {
                            tab_id,
                            foreign_key_id,
                            result: result.clone(),
                        },
                        cx,
                    );
                    if let AppEvent::Failed(error) = event {
                        this.show_message(error.message, AppMessageKind::Error, cx);
                    }
                    cx.notify();
                });
            });
        });
        self._create_table_reference_columns_tasks
            .insert(task_key, task);
        cx.notify();
    }

    fn create_table_type_select(
        &mut self,
        tab_id: TabId,
        column_id: u64,
        database_kind: DatabaseKind,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<SearchableVec<CreateTableTypeOption>>> {
        let key = (tab_id, column_id);
        if !self.create_table_type_selects.contains_key(&key) {
            let (options, selected_index) =
                create_table_type_select_items(database_kind, value);
            let select = cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(options),
                    selected_index,
                    window,
                    cx,
                )
                .searchable(true)
            });
            let subscription = cx.subscribe_in(
                &select,
                window,
                move |this: &mut NavicatMain,
                      _select,
                      event: &SelectEvent<SearchableVec<CreateTableTypeOption>>,
                      _window,
                      cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let Some(value) = value {
                        if value.is_empty() {
                            this.create_table_custom_type_editing = Some((tab_id, column_id));
                            cx.notify();
                            return;
                        }
                        this.set_create_table_column_input(
                            tab_id,
                            column_id,
                            CreateTableColumnField::DataType,
                            value.clone(),
                            cx,
                        );
                    }
                },
            );
            self.create_table_type_selects.insert(key, select);
            self._create_table_type_select_subscriptions
                .insert(key, subscription);
        }

        let select = self.create_table_type_selects.get(&key).cloned().unwrap();
        let selected_matches = select
            .read(cx)
            .selected_value()
            .is_some_and(|selected| selected.trim().eq_ignore_ascii_case(value.trim()));
        if !selected_matches {
            let (options, selected_index) =
                create_table_type_select_items(database_kind, value);
            select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(options), window, cx);
                select.set_selected_index(selected_index, window, cx);
            });
        }
        select
    }

    fn create_table_select(
        &mut self,
        key: CreateTableSelectKey,
        options: Vec<String>,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<SearchableVec<CreateTableSelectOption>>> {
        let inherit_default = matches!(
            key,
            CreateTableSelectKey::ColumnCharset(..)
                | CreateTableSelectKey::ColumnCollation(..)
                | CreateTableSelectKey::TableEngine(..)
                | CreateTableSelectKey::TableCharset(..)
                | CreateTableSelectKey::TableCollation(..)
                | CreateTableSelectKey::TableRowFormat(..)
        );
        if !self.create_table_selects.contains_key(&key) {
            let selected_index = create_table_select_index(&options, value);
            let select_options = options.clone();
            let select = cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(
                        select_options
                            .iter()
                            .cloned()
                            .map(|value| CreateTableSelectOption {
                                value,
                                inherit_default,
                            })
                            .collect::<Vec<_>>(),
                    ),
                    selected_index,
                    window,
                    cx,
                )
            });
            let subscription = cx.subscribe_in(
                &select,
                window,
                move |this: &mut NavicatMain,
                      _select,
                      event: &SelectEvent<SearchableVec<CreateTableSelectOption>>,
                      _window,
                      cx| {
                    let SelectEvent::Confirm(value) = event;
                    let value = value.as_ref().cloned().unwrap_or_default();
                    match key {
                        CreateTableSelectKey::ColumnCharset(tab_id, column_id) => {
                            this.set_create_table_column_input(
                                tab_id,
                                column_id,
                                CreateTableColumnField::Charset,
                                value.clone(),
                                cx,
                            );
                            let selected_charset = value.trim();
                            let collations = create_database_collation_options(selected_charset);
                            let current_collation = this
                                .controller
                                .state()
                                .tabs
                                .iter()
                                .find_map(|tab| match &tab.kind {
                                    TabKind::CreateTable(create) if tab.id == tab_id => create
                                        .columns
                                        .iter()
                                        .find(|column| column.id == column_id)
                                        .map(|column| column.collation.clone()),
                                    _ => None,
                                })
                                .unwrap_or_default();
                            if !selected_charset.is_empty()
                                && !current_collation.trim().is_empty()
                                && !collations
                                    .iter()
                                    .any(|item| item == current_collation.trim())
                            {
                                this.set_create_table_column_input(
                                    tab_id,
                                    column_id,
                                    CreateTableColumnField::Collation,
                                    default_collation_for_charset(selected_charset).to_string(),
                                    cx,
                                );
                            }
                        }
                        CreateTableSelectKey::ColumnCollation(tab_id, column_id) => {
                            this.set_create_table_column_input(
                                tab_id,
                                column_id,
                                CreateTableColumnField::Collation,
                                value.clone(),
                                cx,
                            );
                        }
                        CreateTableSelectKey::IndexType(tab_id, index_id) => {
                            this.dispatch(
                                AppCommand::SetCreateTableIndexField {
                                    tab_id,
                                    index_id,
                                    field: CreateTableIndexField::IndexType,
                                    value: value.clone(),
                                },
                                cx,
                            );
                        }
                        CreateTableSelectKey::IndexMethod(tab_id, index_id) => {
                            this.dispatch(
                                AppCommand::SetCreateTableIndexField {
                                    tab_id,
                                    index_id,
                                    field: CreateTableIndexField::IndexMethod,
                                    value: value.clone(),
                                },
                                cx,
                            );
                        }
                        CreateTableSelectKey::ForeignKeyReferencedDatabase(
                            tab_id,
                            foreign_key_id,
                        ) => {
                            this.dispatch(
                                AppCommand::SetCreateTableForeignKeyField {
                                    tab_id,
                                    foreign_key_id,
                                    field: CreateTableForeignKeyField::ReferencedDatabase,
                                    value: value.clone(),
                                },
                                cx,
                            );
                            if !value.trim().is_empty()
                                && let Some(database_path) = create_table_reference_database_path(
                                    this.controller.state(),
                                    tab_id,
                                    &value,
                                )
                            {
                                let database_key =
                                    database_tree_key(database_path.connection_id, &value);
                                if !this.loaded_database_children.contains(&database_key)
                                    && !this.loading_databases.contains(&database_key)
                                {
                                    this.load_database_children(database_path, database_key, cx);
                                }
                            }
                        }
                        CreateTableSelectKey::ForeignKeyReferencedTable(tab_id, foreign_key_id) => {
                            this.dispatch(
                                AppCommand::SetCreateTableForeignKeyField {
                                    tab_id,
                                    foreign_key_id,
                                    field: CreateTableForeignKeyField::ReferencedTable,
                                    value: value.clone(),
                                },
                                cx,
                            );
                            this.start_create_table_reference_columns_load(
                                tab_id,
                                foreign_key_id,
                                cx,
                            );
                        }
                        CreateTableSelectKey::ForeignKeyOnDelete(tab_id, foreign_key_id) => {
                            this.dispatch(
                                AppCommand::SetCreateTableForeignKeyField {
                                    tab_id,
                                    foreign_key_id,
                                    field: CreateTableForeignKeyField::OnDelete,
                                    value: value.clone(),
                                },
                                cx,
                            );
                        }
                        CreateTableSelectKey::ForeignKeyOnUpdate(tab_id, foreign_key_id) => {
                            this.dispatch(
                                AppCommand::SetCreateTableForeignKeyField {
                                    tab_id,
                                    foreign_key_id,
                                    field: CreateTableForeignKeyField::OnUpdate,
                                    value: value.clone(),
                                },
                                cx,
                            );
                        }
                        CreateTableSelectKey::TriggerTiming(tab_id, trigger_id) => {
                            this.dispatch(
                                AppCommand::SetCreateTableTriggerField {
                                    tab_id,
                                    trigger_id,
                                    field: CreateTableTriggerField::Timing,
                                    value: value.clone(),
                                },
                                cx,
                            );
                        }
                        CreateTableSelectKey::TableEngine(tab_id) => {
                            this.set_create_table_option_input(
                                tab_id,
                                CreateTableOptionField::Engine,
                                value.clone(),
                                cx,
                            );
                        }
                        CreateTableSelectKey::TableCharset(tab_id) => {
                            this.set_create_table_option_input(
                                tab_id,
                                CreateTableOptionField::Charset,
                                value.clone(),
                                cx,
                            );
                            let selected_charset = value.trim();
                            let collations = create_database_collation_options(selected_charset);
                            let current_collation = this
                                .controller
                                .state()
                                .tabs
                                .iter()
                                .find_map(|tab| match &tab.kind {
                                    TabKind::CreateTable(create) if tab.id == tab_id => {
                                        Some(create.collation.clone())
                                    }
                                    _ => None,
                                })
                                .unwrap_or_default();
                            if !selected_charset.is_empty()
                                && !current_collation.trim().is_empty()
                                && !collations
                                    .iter()
                                    .any(|item| item == current_collation.trim())
                            {
                                this.set_create_table_option_input(
                                    tab_id,
                                    CreateTableOptionField::Collation,
                                    default_collation_for_charset(selected_charset).to_string(),
                                    cx,
                                );
                            }
                        }
                        CreateTableSelectKey::TableCollation(tab_id) => {
                            this.set_create_table_option_input(
                                tab_id,
                                CreateTableOptionField::Collation,
                                value.clone(),
                                cx,
                            );
                        }
                        CreateTableSelectKey::TableRowFormat(tab_id) => {
                            this.set_create_table_option_input(
                                tab_id,
                                CreateTableOptionField::RowFormat,
                                value.clone(),
                                cx,
                            );
                        }
                        CreateTableSelectKey::TablePartitionMethod(tab_id) => {
                            this.dispatch(
                                AppCommand::SetCreateTablePartitionField {
                                    tab_id,
                                    field: CreateTablePartitionField::Method,
                                    value: value.clone(),
                                },
                                cx,
                            );
                        }
                    }
                },
            );
            self.create_table_selects.insert(key, select);
            self._create_table_select_subscriptions
                .insert(key, subscription);
        }

        let select = self.create_table_selects.get(&key).cloned().unwrap();
        let selected_index = create_table_select_index(&options, value);
        select.update(cx, |select, cx| {
            select.set_items(
                SearchableVec::new(
                    options
                        .into_iter()
                        .map(|value| CreateTableSelectOption {
                            value,
                            inherit_default,
                        })
                        .collect::<Vec<_>>(),
                ),
                window,
                cx,
            );
            select.set_selected_index(selected_index, window, cx);
        });
        select
    }
}

fn create_table_content(
    tab_id: TabId,
    create: &CreateTableState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut content = div()
        .flex_1()
        .min_h(px(0.))
        .relative()
        .bg(colors.content_bg)
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(create_table_header(tab_id, create, this, window, colors, cx))
        .child(create_table_editor(tab_id, create, this, window, colors, cx));
    if create.details_loading {
        content = content.child(create_table_loading_overlay(colors));
    }
    content
}

fn create_table_loading_overlay(colors: UiColors) -> Div {
    div()
        .debug_selector(|| "create-table-details-loading".into())
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.64)
        } else {
            hsla(210. / 360., 0.20, 0.98, 0.72)
        })
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_3()
                .child(loading_spinner_with_color(32., colors.muted))
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.text)
                        .child(tr("app.create_table.loading_design")),
                ),
        )
}

fn create_table_header(
    tab_id: TabId,
    create: &CreateTableState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let name_input = this.create_table_input(
        CreateTableInputKey::TableName(tab_id),
        tr("app.create_table.placeholder.table_name"),
        &create.table_name,
        window,
        cx,
    );
    let comment_input = this.create_table_input(
        CreateTableInputKey::TableComment(tab_id),
        tr("app.create_table.placeholder.table_comment"),
        &create.comment,
        window,
        cx,
    );

    div()
        .flex_none()
        .px_4()
        .py_3()
        .flex()
        .items_start()
        .justify_between()
        .gap_4()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(create_table_labeled_input(tr("app.create_table.table_name"), name_input, 360., window, colors, cx))
                .child(create_table_labeled_input(
                    tr("app.create_table.comment"),
                    comment_input,
                    520.,
                    window,
                    colors,
                    cx,
                )),
        )
        .child(create_table_header_actions(tab_id, create, cx))
}

fn create_table_header_actions(
    tab_id: TabId,
    create: &CreateTableState,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let validation_error = create.validation_error();
    let save_disabled = validation_error.is_some() || create.applying || create.details_loading;
    let save_tooltip = if create.applying {
        tr("app.create_table.saving").to_string()
    } else if create.details_loading {
        tr("app.create_table.loading_design").to_string()
    } else if create.is_design() {
        validation_error
            .map(str::to_string)
            .unwrap_or_else(|| tr("app.create_table.save_design").to_string())
    } else {
        validation_error
            .map(str::to_string)
            .unwrap_or_else(|| tr("app.create_table.save_and_create").to_string())
    };
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .child(
            Button::new("create-table-cancel")
                .label(tr("app.common.cancel"))
                .small()
                .outline()
                .disabled(create.applying)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch(AppCommand::CloseTab(tab_id), cx);
                })),
        )
        .child(
            Button::new("create-table-save")
                .label(if create.applying {
                    tr("app.create_table.saving_short")
                } else {
                    tr("app.common.save")
                })
                .small()
                .primary()
                .disabled(save_disabled)
                .tooltip(save_tooltip)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.start_create_table_apply(tab_id, cx);
                })),
        )
}

fn create_table_editor(
    tab_id: TabId,
    create: &CreateTableState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .debug_selector(|| "create-table-editor".into())
        .flex_1()
        .min_h(px(CREATE_TABLE_EDITOR_MIN_HEIGHT))
        .mx_4()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(create_table_tab_bar(tab_id, create, colors, cx))
        .child(match create.active_tab {
            CreateTableTab::Fields => create_table_fields(tab_id, create, this, window, colors, cx),
            CreateTableTab::Indexes => {
                create_table_indexes(tab_id, create, this, window, colors, cx)
            }
            CreateTableTab::ForeignKeys => {
                create_table_foreign_keys(tab_id, create, this, window, colors, cx)
            }
            CreateTableTab::Checks => create_table_checks(tab_id, create, this, window, colors, cx),
            CreateTableTab::Triggers => {
                create_table_triggers(tab_id, create, this, window, colors, cx)
            }
            CreateTableTab::Options => create_table_options(tab_id, create, this, window, colors, cx),
            CreateTableTab::Partitions => {
                create_table_partitions(tab_id, create, this, window, colors, cx)
            }
            // 两个预览页签都用底层编辑器渲染，需要宿主当前主题派生的编辑器配色。
            CreateTableTab::SqlPreview => {
                let editor_theme = this.editor_theme_for(cx);
                create_table_sql_preview(tab_id, create, editor_theme, window, colors, cx)
            }
            CreateTableTab::Ddl => {
                let editor_theme = this.editor_theme_for(cx);
                create_table_ddl_preview(tab_id, create, editor_theme, window, colors, cx)
            }
        })
}

fn create_table_tab_bar(
    tab_id: TabId,
    create: &CreateTableState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let active_tab = create.active_tab;
    let mut tabs = div().flex().items_center().gap_1();
    for (tab, label) in [
        (CreateTableTab::Fields, tr("app.create_table.tab.fields")),
        (CreateTableTab::Indexes, tr("app.create_table.tab.indexes")),
        (CreateTableTab::ForeignKeys, tr("app.create_table.tab.foreign_keys")),
        (CreateTableTab::Checks, tr("app.create_table.tab.checks")),
        (CreateTableTab::Triggers, tr("app.create_table.tab.triggers")),
        (CreateTableTab::Options, tr("app.create_table.tab.options")),
        (CreateTableTab::Partitions, tr("app.create_table.tab.partitions")),
        (CreateTableTab::SqlPreview, tr("app.create_table.tab.sql_preview")),
    ] {
        tabs = tabs.child(create_table_tab_button(tab_id, tab, label, active_tab == tab, colors, cx));
    }
    if create.is_design() {
        tabs = tabs.child(create_table_tab_button(
            tab_id,
            CreateTableTab::Ddl,
            "DDL",
            active_tab == CreateTableTab::Ddl,
            colors,
            cx,
        ));
    }

    div()
        .h(px(40.))
        .px_3()
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .justify_between()
        .child(tabs)
        .when(active_tab == CreateTableTab::Fields, |this| {
            this.child(create_table_tab_action(
                tr("app.create_table.add_field"),
                AppCommand::AddCreateTableColumn(tab_id),
                colors,
                cx,
            ))
        })
        .when(active_tab == CreateTableTab::Indexes, |this| {
            this.child(create_table_tab_action(
                tr("app.create_table.add_index"),
                AppCommand::AddCreateTableIndex(tab_id),
                colors,
                cx,
            ))
        })
        .when(active_tab == CreateTableTab::ForeignKeys, |this| {
            this.child(create_table_tab_action(
                tr("app.create_table.add_foreign_key"),
                AppCommand::AddCreateTableForeignKey(tab_id),
                colors,
                cx,
            ))
        })
        .when(active_tab == CreateTableTab::Checks, |this| {
            this.child(create_table_tab_action(
                tr("app.create_table.add_check"),
                AppCommand::AddCreateTableCheck(tab_id),
                colors,
                cx,
            ))
        })
        .when(active_tab == CreateTableTab::Triggers && create.database_kind != DatabaseKind::Dameng, |this| {
            this.child(create_table_tab_action(
                tr("app.create_table.add_trigger"),
                AppCommand::AddCreateTableTrigger(tab_id),
                colors,
                cx,
            ))
        })
}

fn create_table_tab_action(
    label: impl Into<SharedString>,
    command: AppCommand,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label = label.into();
    div()
        .debug_selector({
            let label = label.clone();
            move || format!("create-table-add:{label}")
        })
        .h(px(28.))
        .px_3()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(command.clone(), cx);
                cx.stop_propagation();
            }),
        )
        .child(app_icon(AppIcon::Plus, 15., colors.text))
        .child(
            div()
                .text_size(ui_text_size(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(label),
        )
}

fn create_table_add_row_command(tab_id: TabId, create: &CreateTableState) -> Option<AppCommand> {
    match create.active_tab {
        CreateTableTab::Fields => Some(AppCommand::AddCreateTableColumn(tab_id)),
        CreateTableTab::Indexes => Some(AppCommand::AddCreateTableIndex(tab_id)),
        CreateTableTab::ForeignKeys => Some(AppCommand::AddCreateTableForeignKey(tab_id)),
        CreateTableTab::Checks => Some(AppCommand::AddCreateTableCheck(tab_id)),
        CreateTableTab::Triggers if create.database_kind == DatabaseKind::Dameng => None,
        CreateTableTab::Triggers => Some(AppCommand::AddCreateTableTrigger(tab_id)),
        CreateTableTab::Options
        | CreateTableTab::Partitions
        | CreateTableTab::SqlPreview
        | CreateTableTab::Ddl => None,
    }
}

fn create_table_tab_button(
    tab_id: TabId,
    create_tab: CreateTableTab,
    label: impl Into<SharedString>,
    active: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label = label.into();
    div().debug_selector(move || format!("create-table-tab:{create_tab:?}"))
        .h(px(28.))
        .px_3()
        .rounded(colors.radius_lg)
        .flex()
        .items_center()
        .cursor_pointer()
        .bg(if active { colors.hover } else { colors.panel_alt })
        .text_color(if active { colors.text } else { colors.muted })
        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(
                    AppCommand::SelectCreateTableTab {
                        tab_id,
                        create_tab,
                    },
                    cx,
                );
                cx.stop_propagation();
            }),
        )
        .child(
            div().debug_selector(move || format!("create-table-tab:{create_tab:?}"))
                .text_size(ui_text_size(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(label),
        )
}

fn create_table_fields(
    tab_id: TabId,
    create: &CreateTableState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut rows = div().flex().flex_col();
    let row_count = create.columns.len();
    for (index, column) in create.columns.iter().enumerate() {
        let row = CreateTableFieldsTableRow {
            index,
            column: column.clone(),
            name_input: this.create_table_input(
                CreateTableInputKey::ColumnName(tab_id, column.id),
                tr("app.create_table.field.name"),
                &column.name,
                window,
                cx,
            ),
            type_select: this.create_table_type_select(
                tab_id,
                column.id,
                create.database_kind,
                &column.data_type,
                window,
                cx,
            ),
            type_input: this.create_table_input(
                CreateTableInputKey::ColumnCustomType(tab_id, column.id),
                tr("app.create_table.column.custom_type_placeholder"),
                &column.data_type,
                window,
                cx,
            ),
            length_input: this.create_table_input(
                CreateTableInputKey::ColumnLength(tab_id, column.id),
                create_table_length_placeholder(create.database_kind, &column.data_type),
                &column.length,
                window,
                cx,
            ),
            default_input: this.create_table_input(
                CreateTableInputKey::ColumnDefault(tab_id, column.id),
                tr("app.create_table.field.default"),
                &column.default_value,
                window,
                cx,
            ),
            comment_input: this.create_table_input(
                CreateTableInputKey::ColumnComment(tab_id, column.id),
                tr("app.create_table.field.comment"),
                &column.comment,
                window,
                cx,
            ),
            comment_editor_input: this.create_table_input(
                CreateTableInputKey::ColumnCommentEditor(tab_id, column.id),
                tr("app.create_table.field_placeholder"),
                &column.comment,
                window,
                cx,
            ),
        };
        rows = rows.child(create_table_field_row(
            cx.entity().downgrade(),
            tab_id,
            create.database_kind,
            row,
            row_count,
            create.selected_column_id == Some(column.id),
            this.create_table_custom_type_editing == Some((tab_id, column.id)),
            this.create_table_comment_editor_sizes
                .get(&(tab_id, column.id))
                .copied()
                .unwrap_or((
                    CREATE_TABLE_COMMENT_EDITOR_DEFAULT_WIDTH,
                    CREATE_TABLE_COMMENT_EDITOR_DEFAULT_HEIGHT,
                )),
            window,
            colors,
            cx,
        ));
    }

    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_scrollbar()
                .child(
                    div()
                        .min_w(px(CREATE_TABLE_FIELDS_TABLE_WIDTH))
                        .h_full()
                        .flex()
                        .flex_col()
                        .child(create_table_field_header_row(colors))
                        .child(
                            div()
                                .flex_1()
                                .min_h(px(0.))
                                .overflow_y_scrollbar()
                                .child(rows),
                        ),
                ),
        )
        .child(create_table_column_options(
            tab_id,
            create.database_kind,
            create.selected_column(),
            create.selected_column().and_then(|selected| match &create.mode {
                CreateTableMode::Design { original, .. } => original
                    .columns
                    .iter()
                    .find(|column| column.id == selected.id),
                CreateTableMode::Create => None,
            }),
            this,
            window,
            colors,
            cx,
        ))
}

fn create_table_field_header_row(colors: UiColors) -> Div {
    div()
        .h(px(32.))
        .flex_none()
        .flex_shrink_0()
        .bg(colors.panel_alt)
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .flex()
        .items_center()
        .child(create_table_field_header_cell("#", 46., colors))
        .child(create_table_field_header_cell(
            tr("app.create_table.field.name"),
            190.,
            colors,
        ))
        .child(create_table_field_header_cell(
            tr("app.create_table.field.type"),
            150.,
            colors,
        ))
        .child(create_table_field_header_cell(
            tr("app.create_table.field.length"),
            100.,
            colors,
        ))
        .child(create_table_field_header_cell(
            tr("app.create_table.field.nullable"),
            86.,
            colors,
        ))
        .child(create_table_field_header_cell(
            tr("app.create_table.field.primary_key"),
            68.,
            colors,
        ))
        .child(create_table_field_header_cell(
            tr("app.create_table.field.default"),
            160.,
            colors,
        ))
        .child(create_table_field_header_cell(
            tr("app.create_table.field.comment"),
            180.,
            colors,
        ))
        .child(create_table_field_header_cell(
            tr("app.create_table.field.operations"),
            CREATE_TABLE_OPERATIONS_WIDTH,
            colors,
        ))
}

fn create_table_field_header_cell(
    label: impl Into<SharedString>,
    width: f32,
    colors: UiColors,
) -> Div {
    let label = label.into();
    div()
        .debug_selector({
            let label = label.clone();
            move || format!("create-table-heading:{label}")
        })
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_3()
        .flex()
        .items_center()
        .child(label)
}

fn create_table_field_row(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    database_kind: DatabaseKind,
    row: CreateTableFieldsTableRow,
    row_count: usize,
    selected: bool,
    editing_type: bool,
    comment_editor_size: (f32, f32),
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let select_view = view.clone();
    let column_id = row.column.id;
    let row_index = row.index;
    let type_value = row.column.data_type.clone();
    let length_enabled = create_table_supports_length(database_kind, &row.column.data_type);
    div()
        .h(px(34.))
        .flex_none()
        .flex_shrink_0()
        .bg(if selected { colors.hover } else { colors.panel_bg })
        .flex()
        .items_center()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            let _ = select_view.update(cx, |this, cx| {
                this.dispatch(
                    AppCommand::SelectCreateTableColumn { tab_id, column_id },
                    cx,
                );
            });
            cx.stop_propagation();
        })
        .child(
            create_table_table_cell((row.index + 1).to_string(), colors)
                .debug_selector(move || format!("create-table-row-{row_index}"))
                .w(px(46.))
                .flex_shrink_0()
                .px_3(),
        )
        .child(create_table_input_cell(
            row.name_input,
            190.,
            window,
            colors,
            cx,
        ))
        .child(create_table_type_select_cell(
            row.type_select,
            row.type_input,
            150.,
            row_index,
            editing_type,
            tab_id,
            column_id,
            &type_value,
            window,
            colors,
            cx,
        ))
        .child(
            create_table_input_cell_enabled(
                row.length_input,
                100.,
                length_enabled,
                window,
                colors,
                cx,
            )
            .debug_selector(move || {
                format!("create-table-length-{row_index}-enabled:{length_enabled}")
            }),
        )
        .child(create_table_flag_cell(
            view.clone(),
            tab_id,
            column_id,
            tr("app.create_table.boolean_yes"),
            row.column.nullable,
            CreateTableColumnFlag::Nullable,
            86.,
            colors,
        ))
        .child(create_table_flag_cell(
            view.clone(),
            tab_id,
            column_id,
            "",
            row.column.primary_key,
            CreateTableColumnFlag::PrimaryKey,
            68.,
            colors,
        ))
        .child(create_table_default_cell(
            view.clone(),
            tab_id,
            column_id,
            row.default_input,
            row.column.default_value.as_str(),
            160.,
            window,
            colors,
            cx,
        ))
        .child(create_table_comment_cell(
            view.clone(),
            tab_id,
            column_id,
            row.comment_input,
            row.comment_editor_input,
            comment_editor_size,
            180.,
            window,
            colors,
            cx,
        ))
        .child(create_table_operations_cell(
            view,
            tab_id,
            column_id,
            row.index > 0,
            row.index + 1 < row_count,
            colors,
        ))
}

fn create_table_available_index_fields(create: &CreateTableState) -> Vec<String> {
    create
        .columns
        .iter()
        .map(|column| column.name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

fn create_table_primary_index_fields(create: &CreateTableState) -> Option<String> {
    let parts = create
        .columns
        .iter()
        .filter(|column| column.primary_key && !column.name.trim().is_empty())
        .map(|column| {
            let mut part = fluxdb_core::MySqlDialect.quote_identifier(column.name.trim());
            if column.primary_key && !column.key_length.trim().is_empty() {
                part.push('(');
                part.push_str(column.key_length.trim());
                part.push(')');
            }
            part.push_str(" ASC");
            part
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

fn create_table_index_type_options() -> Vec<String> {
    ["", "NORMAL", "UNIQUE", "FULLTEXT", "SPATIAL"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn create_table_index_method_options() -> Vec<String> {
    ["", "BTREE", "HASH"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn create_table_index_method_options_for_type(index_type: &str) -> Vec<String> {
    if create_table_index_type_supports_method(index_type) {
        create_table_index_method_options()
    } else {
        vec![String::new()]
    }
}

fn create_table_index_type_supports_method(index_type: &str) -> bool {
    !matches!(
        index_type.trim().to_ascii_uppercase().as_str(),
        "FULLTEXT" | "SPATIAL"
    )
}

fn create_table_sort_order_options() -> Vec<String> {
    ["", "ASC", "DESC"].into_iter().map(str::to_string).collect()
}

fn create_table_readonly_cell(text: impl Into<String>, width: f32, strong: bool, colors: UiColors) -> Div {
    create_table_table_cell(text, colors)
        .w(px(width))
        .flex_shrink_0()
        .px_3()
        .font_weight(if strong {
            gpui::FontWeight::SEMIBOLD
        } else {
            gpui::FontWeight::NORMAL
        })
}

fn create_table_index_fields_cell(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    index: CreateTableIndex,
    field_rows: Vec<CreateTableIndexFieldEditorRow>,
    available_fields: Vec<String>,
    width: f32,
    colors: UiColors,
) -> Div {
    let summary = create_table_index_fields_summary(&index);
    div()
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_2()
        .flex()
        .items_center()
        .child(
            Popover::new((
                gpui::ElementId::Name(format!("create-table-index-fields-{}", tab_id.0).into()),
                index.id.to_string(),
            ))
            .appearance(false)
            .anchor(Anchor::TopRight)
            .trigger(
                Button::new((
                    gpui::ElementId::Name(
                        format!("create-table-index-fields-trigger-{}", tab_id.0).into(),
                    ),
                    index.id.to_string(),
                ))
                .ghost()
                .xsmall()
                .w(px(width - 18.))
                .h(px(CREATE_TABLE_INPUT_HEIGHT))
                .p_0()
                .child(
                    div()
                    .size_full()
                    .w(px(width - 18.))
                    .h(px(CREATE_TABLE_INPUT_HEIGHT))
                    .rounded(colors.radius)
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.input_bg)
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .hover(move |style| style.border_color(create_table_input_hover_border_color(false, colors)))
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .px_2()
                            .text_size(ui_text_size(13.))
                            .text_color(colors.text)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(if summary.is_empty() {
                                tr("app.create_table.error.select_field").to_string()
                            } else {
                                summary
                            }),
                    )
                    .child(
                        div()
                            .w(px(24.))
                            .h_full()
                            .border_l_1()
                            .border_color(colors.border_soft)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(app_icon(AppIcon::ChevronDown, 13., colors.muted)),
                    ),
                ),
            )
            .content(move |_, window, cx| {
                create_table_index_fields_popover(
                    view.clone(),
                    tab_id,
                    index.id,
                    field_rows.clone(),
                    available_fields.clone(),
                    colors,
                    window,
                    cx,
                )
            }),
        )
}

fn create_table_index_fields_popover(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    index_id: u64,
    field_rows: Vec<CreateTableIndexFieldEditorRow>,
    available_fields: Vec<String>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> Div {
    let popover = cx.entity();
    let (active_dropdown, stored_selection) = view
        .upgrade()
        .map(|view| {
            let this = view.read(cx);
            (
                this.create_table_index_field_dropdown,
                this.create_table_index_field_selection,
            )
        })
        .unwrap_or((None, None));
    let field_count = field_rows.len();
    let selected_column_index = stored_selection
        .filter(|selection| {
            selection.tab_id == tab_id
                && selection.index_id == index_id
                && selection.column_index < field_count
        })
        .map(|selection| selection.column_index)
        .or_else(|| (field_count > 0).then_some(0));
    let can_move_up = selected_column_index.is_some_and(|column_index| column_index > 0);
    let can_move_down =
        selected_column_index.is_some_and(|column_index| column_index + 1 < field_count);
    let can_remove = selected_column_index.is_some();
    let can_add = create_table_has_unused_index_fields(&field_rows, &available_fields);
    let active_menu = active_dropdown
        .filter(|key| key.tab_id == tab_id && key.index_id == index_id)
        .and_then(|key| {
            field_rows
                .iter()
                .find(|row| row.column_index == key.column_index)
                .map(|row| (key, row.name.clone(), row.sort_order.clone()))
        });
    let move_up_view = view.clone();
    let move_down_view = view.clone();
    let add_view = view.clone();
    let remove_view = view.clone();
    let mut table = div()
        .w_full()
        .border_1()
        .border_color(colors.border_soft)
        .overflow_hidden();
    table = table.child(create_table_index_fields_header(colors));
    if field_rows.is_empty() {
        table = table.child(
            div()
                .h(px(30.))
                .px_3()
                .flex()
                .items_center()
                .text_size(ui_text_size(13.))
                .text_color(colors.muted)
                .child(tr("app.create_table.error.add_field")),
        );
    } else {
        for row in field_rows.clone() {
            let selected = selected_column_index == Some(row.column_index);
            table = table.child(create_table_index_field_editor_row(
                row,
                view.clone(),
                tab_id,
                active_dropdown,
                selected,
                colors,
                window,
                cx,
            ));
        }
    }

    let dropdown_open = active_menu.is_some();
    let panel = div()
        .w(px(CREATE_TABLE_INDEX_FIELDS_POPOVER_WIDTH))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow_md()
        .overflow_hidden()
        .p_1()
        .flex()
        .flex_col()
        .gap_1()
        .child(table)
        .child(
            div()
                .h(px(30.))
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(create_table_index_field_icon_button(
                            AppIcon::ChevronUp,
                            tr("app.create_table.popover.move_up"),
                            can_move_up,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            move |_, _, cx| {
                                if let Some(column_index) = selected_column_index {
                                    let _ = move_up_view.update(cx, |this, cx| {
                                        this.dispatch(
                                            AppCommand::MoveCreateTableIndexColumnUp {
                                                tab_id,
                                                index_id,
                                                column_index,
                                            },
                                            cx,
                                        );
                                        this.create_table_index_field_selection =
                                            Some(CreateTableIndexFieldSelectionKey {
                                                tab_id,
                                                index_id,
                                                column_index: column_index.saturating_sub(1),
                                            });
                                        this.create_table_index_field_dropdown = None;
                                        cx.notify();
                                    });
                                }
                                cx.stop_propagation();
                            },
                        ))
                        .child(create_table_index_field_icon_button(
                            AppIcon::ChevronDown,
                            tr("app.create_table.popover.move_down"),
                            can_move_down,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            move |_, _, cx| {
                                if let Some(column_index) = selected_column_index {
                                    let _ = move_down_view.update(cx, |this, cx| {
                                        this.dispatch(
                                            AppCommand::MoveCreateTableIndexColumnDown {
                                                tab_id,
                                                index_id,
                                                column_index,
                                            },
                                            cx,
                                        );
                                        this.create_table_index_field_selection =
                                            Some(CreateTableIndexFieldSelectionKey {
                                                tab_id,
                                                index_id,
                                                column_index: column_index + 1,
                                            });
                                        this.create_table_index_field_dropdown = None;
                                        cx.notify();
                                    });
                                }
                                cx.stop_propagation();
                            },
                        ))
                        .child(create_table_index_field_icon_button(
                            AppIcon::Plus,
                            tr("app.create_table.popover.add_field"),
                            can_add,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            move |_, _, cx| {
                                if can_add {
                                    let _ = add_view.update(cx, |this, cx| {
                                        this.dispatch(
                                            AppCommand::AddCreateTableIndexColumn {
                                                tab_id,
                                                index_id,
                                            },
                                            cx,
                                        );
                                        this.create_table_index_field_selection =
                                            Some(CreateTableIndexFieldSelectionKey {
                                                tab_id,
                                                index_id,
                                                column_index: field_count,
                                            });
                                        this.create_table_index_field_dropdown = None;
                                        cx.notify();
                                    });
                                }
                                cx.stop_propagation();
                            },
                        ))
                        .child(create_table_index_field_icon_button(
                            AppIcon::Minus,
                            tr("app.create_table.popover.remove_last_field"),
                            can_remove,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            move |_, _, cx| {
                                if let Some(column_index) = selected_column_index {
                                    let _ = remove_view.update(cx, |this, cx| {
                                        this.dispatch(
                                            AppCommand::RemoveCreateTableIndexColumn {
                                                tab_id,
                                                index_id,
                                                column_index,
                                            },
                                            cx,
                                        );
                                        this.create_table_index_field_selection =
                                            (field_count > 1).then_some(
                                                CreateTableIndexFieldSelectionKey {
                                                    tab_id,
                                                    index_id,
                                                    column_index: column_index
                                                        .min(field_count.saturating_sub(2)),
                                                },
                                            );
                                        this.create_table_index_field_dropdown = None;
                                        cx.notify();
                                    });
                                }
                                cx.stop_propagation();
                            },
                        )),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(create_table_popover_text_button(tr("app.create_table.popover.confirm"), colors).on_mouse_down(
                            MouseButton::Left,
                            {
                                let popover = popover.clone();
                                move |_, window, cx| {
                                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                                    cx.stop_propagation();
                                }
                            },
                        ))
                        .child(create_table_popover_text_button(tr("app.create_table.popover.cancel"), colors).on_mouse_down(
                            MouseButton::Left,
                            move |_, window, cx| {
                                popover.update(cx, |state, cx| state.dismiss(window, cx));
                                cx.stop_propagation();
                            },
                        )),
                ),
        );

    let content = div()
        .relative()
        .w(px(CREATE_TABLE_INDEX_FIELDS_POPOVER_WIDTH))
        .child(panel)
        .when(dropdown_open, |this| {
            // 外层透明区域纳入下拉菜单的点击命中范围，避免 Popover 把菜单点击当成外部点击关闭。
            this.child(div().h(px(CREATE_TABLE_INDEX_FIELD_DROPDOWN_EXTRA_HEIGHT)))
        });

    if let Some((key, name, sort_order)) = active_menu {
        let (field, value, options, disabled_options, width, kind) = match key.kind {
            CreateTableIndexFieldDropdownKind::Name => (
                CreateTableIndexColumnField::Name,
                name,
                create_table_index_field_name_options(available_fields),
                create_table_used_index_field_names(&field_rows, key.column_index),
                CREATE_TABLE_INDEX_FIELD_NAME_WIDTH,
                CreateTableIndexFieldDropdownKind::Name,
            ),
            CreateTableIndexFieldDropdownKind::SortOrder => (
                CreateTableIndexColumnField::SortOrder,
                sort_order,
                create_table_sort_order_options(),
                BTreeSet::new(),
                CREATE_TABLE_INDEX_FIELD_SORT_ORDER_WIDTH,
                CreateTableIndexFieldDropdownKind::SortOrder,
            ),
        };
        content.child(create_table_index_field_floating_menu(
            view,
            key,
            field,
            value,
            options,
            disabled_options,
            width,
            kind,
            colors,
        ))
    } else {
        content
    }
}

fn create_table_index_fields_header(colors: UiColors) -> Div {
    div()
        .h(px(26.))
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .text_size(ui_text_size(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(create_table_index_field_header_cell(
            tr("app.create_table.index.field"),
            CREATE_TABLE_INDEX_FIELD_NAME_WIDTH,
            colors,
        ))
        .child(create_table_index_field_header_cell(
            tr("app.create_table.index.sub_part"),
            CREATE_TABLE_INDEX_FIELD_SUB_PART_WIDTH,
            colors,
        ))
        .child(create_table_index_field_header_cell(
            tr("app.create_table.index.sort_order"),
            CREATE_TABLE_INDEX_FIELD_SORT_ORDER_WIDTH,
            colors,
        ))
}

fn create_table_index_field_header_cell(
    label: impl Into<SharedString>,
    width: f32,
    colors: UiColors,
) -> Div {
    let label = label.into();
    div()
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_2()
        .flex()
        .items_center()
        .child(label)
}

fn create_table_index_field_editor_row(
    row: CreateTableIndexFieldEditorRow,
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    active_dropdown: Option<CreateTableIndexFieldDropdownKey>,
    selected: bool,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> Div {
    let name_key = CreateTableIndexFieldDropdownKey {
        tab_id,
        index_id: row.index_id,
        column_index: row.column_index,
        kind: CreateTableIndexFieldDropdownKind::Name,
    };
    let sort_key = CreateTableIndexFieldDropdownKey {
        tab_id,
        index_id: row.index_id,
        column_index: row.column_index,
        kind: CreateTableIndexFieldDropdownKind::SortOrder,
    };
    let selection_key = CreateTableIndexFieldSelectionKey {
        tab_id,
        index_id: row.index_id,
        column_index: row.column_index,
    };
    let select_view = view.clone();

    div()
        .relative()
        .h(px(28.))
        .flex()
        .items_center()
        .bg(colors.panel_bg)
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            let _ = select_view.update(cx, |this, cx| {
                this.create_table_index_field_selection = Some(selection_key);
                cx.notify();
            });
            cx.stop_propagation();
        })
        .child(create_table_index_field_dropdown_cell(
            view.clone(),
            name_key,
            row.name.clone(),
            CREATE_TABLE_INDEX_FIELD_NAME_WIDTH,
            CreateTableIndexFieldDropdownKind::Name,
            active_dropdown == Some(name_key),
            colors,
        ))
        .child(create_table_input_cell(
            row.sub_part_input,
            CREATE_TABLE_INDEX_FIELD_SUB_PART_WIDTH,
            window,
            colors,
            cx,
        ))
        .child(create_table_index_field_dropdown_cell(
            view.clone(),
            sort_key,
            row.sort_order.clone(),
            CREATE_TABLE_INDEX_FIELD_SORT_ORDER_WIDTH,
            CreateTableIndexFieldDropdownKind::SortOrder,
            active_dropdown == Some(sort_key),
            colors,
        ))
        .when(selected, |this| {
            this.child(
                div()
                    .absolute()
                    .left(px(2.))
                    .top(px(5.))
                    .w(px(3.))
                    .h(px(18.))
                    .rounded(colors.radius * 0.5)
                    .bg(create_table_index_field_selection_bg(colors)),
            )
        })
}

fn create_table_index_field_name_options(available_fields: Vec<String>) -> Vec<String> {
    available_fields
}

fn create_table_has_unused_index_fields(
    field_rows: &[CreateTableIndexFieldEditorRow],
    available_fields: &[String],
) -> bool {
    let used = create_table_used_index_field_names(field_rows, usize::MAX);
    available_fields
        .iter()
        .map(|field| field.trim())
        .any(|field| !field.is_empty() && !used.contains(&field.to_ascii_lowercase()))
}

fn create_table_used_index_field_names(
    field_rows: &[CreateTableIndexFieldEditorRow],
    except_column_index: usize,
) -> BTreeSet<String> {
    field_rows
        .iter()
        .filter(|row| row.column_index != except_column_index)
        .map(|row| row.name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn create_table_index_field_dropdown_cell(
    view: WeakEntity<NavicatMain>,
    key: CreateTableIndexFieldDropdownKey,
    value: String,
    width: f32,
    kind: CreateTableIndexFieldDropdownKind,
    open: bool,
    colors: UiColors,
) -> Div {
    let label = create_table_index_field_option_label(&value, kind);
    let menu_width = width - 2.;

    div()
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .child(
            create_table_index_field_dropdown_trigger(
                label,
                !value.trim().is_empty(),
                open,
                menu_width,
                colors,
            )
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.create_table_index_field_selection =
                        Some(CreateTableIndexFieldSelectionKey {
                            tab_id: key.tab_id,
                            index_id: key.index_id,
                            column_index: key.column_index,
                        });
                    this.create_table_index_field_dropdown = if this.create_table_index_field_dropdown
                        == Some(key)
                    {
                        None
                    } else {
                        Some(key)
                    };
                    cx.notify();
                });
                cx.stop_propagation();
            }),
        )
}

fn create_table_index_field_option_label(
    value: &str,
    kind: CreateTableIndexFieldDropdownKind,
) -> String {
    if value.trim().is_empty() {
        match kind {
            CreateTableIndexFieldDropdownKind::Name => String::new(),
            CreateTableIndexFieldDropdownKind::SortOrder => {
                tr("app.create_table.index.unsorted").to_string()
            }
        }
    } else {
        value.to_string()
    }
}

fn create_table_index_field_dropdown_trigger(
    label: String,
    selected: bool,
    open: bool,
    width: f32,
    colors: UiColors,
) -> Div {
    div()
        .w(px(width))
        .h_full()
        .bg(if open || selected {
            colors.input_bg
        } else {
            colors.panel_bg
        })
        .border_1()
        .border_color(if open {
            create_table_input_border_color(true, colors)
        } else {
            colors.border_soft
        })
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_between()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .px_2()
                .flex()
                .items_center()
                .gap_1()
                .text_size(ui_text_size(12.))
                .text_color(if selected {
                    colors.text
                } else {
                    colors.muted
                })
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(label),
                ),
        )
        .child(
            div()
                .w(px(18.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(app_icon(AppIcon::ChevronDown, 12., colors.muted)),
        )
}

fn create_table_index_field_selection_bg(colors: UiColors) -> gpui::Rgba {
    if colors.is_dark {
        rgb(0x1f5f9e)
    } else {
        rgb(0x0078d7)
    }
}

fn create_table_index_field_menu_top(column_index: usize) -> f32 {
    4. + 26. + 28. * (column_index as f32 + 1.) - 1.
}

fn create_table_index_field_menu_left(kind: CreateTableIndexFieldDropdownKind) -> f32 {
    4. + match kind {
        CreateTableIndexFieldDropdownKind::Name => 0.,
        CreateTableIndexFieldDropdownKind::SortOrder => {
            CREATE_TABLE_INDEX_FIELD_NAME_WIDTH + CREATE_TABLE_INDEX_FIELD_SUB_PART_WIDTH
        }
    }
}

fn create_table_index_field_floating_menu(
    view: WeakEntity<NavicatMain>,
    key: CreateTableIndexFieldDropdownKey,
    field: CreateTableIndexColumnField,
    value: String,
    options: Vec<String>,
    disabled_options: BTreeSet<String>,
    width: f32,
    kind: CreateTableIndexFieldDropdownKind,
    colors: UiColors,
) -> Div {
    let mut list = div()
        .w(px(width - 2.))
        .max_h(px(150.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col();

    for option in options {
        let selected = option == value;
        let disabled = disabled_options.contains(&option.trim().to_ascii_lowercase());
        let label = create_table_index_field_option_label(&option, kind);
        let option_value = option.clone();
        let view = view.clone();
        list = list.child(
            div()
                .h(px(24.))
                .flex_none()
                .px_2()
                .flex()
                .items_center()
                .gap_1()
                .text_size(ui_text_size(12.))
                .text_color(if selected {
                    colors.text
                } else if disabled {
                    colors.muted
                } else {
                    colors.text
                })
                .bg(if selected { colors.hover } else { colors.input_bg })
                .opacity(if disabled { 0.48 } else { 1.0 })
                .when(!disabled, |this| {
                    this.cursor_pointer()
                        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
                })
                .child(
                    div()
                        .w(px(12.))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |this| {
                            this.child(app_icon(AppIcon::Check, 10., colors.text))
                        }),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(label),
                )
                .on_mouse_down(MouseButton::Left, {
                    move |_, window, cx| {
                        if disabled {
                            cx.stop_propagation();
                            return;
                        }
                        let _ = view.update(cx, |this, cx| {
                            this.dispatch(
                                AppCommand::SetCreateTableIndexColumnField {
                                    tab_id: key.tab_id,
                                    index_id: key.index_id,
                                    column_index: key.column_index,
                                    field,
                                    value: option_value.clone(),
                                },
                                cx,
                            );
                            this.create_table_index_field_selection =
                                Some(CreateTableIndexFieldSelectionKey {
                                    tab_id: key.tab_id,
                                    index_id: key.index_id,
                                    column_index: key.column_index,
                                });
                            this.create_table_index_field_dropdown = None;
                            cx.notify();
                        });
                        window.refresh();
                        cx.stop_propagation();
                    }
                }),
        );
    }

    div()
        .absolute()
        .top(px(create_table_index_field_menu_top(key.column_index)))
        .left(px(create_table_index_field_menu_left(key.kind)))
        .w(px(width - 2.))
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .w(px(width - 2.))
                .rounded(colors.radius * 0.5)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .shadow_md()
                .overflow_hidden()
                .child(list),
        )
}

fn create_table_index_field_icon_button(
    icon: AppIcon,
    _tooltip: impl Into<SharedString>,
    enabled: bool,
    colors: UiColors,
) -> Div {
    div()
        .size(px(26.))
        .rounded(colors.radius)
        .opacity(if enabled { 1.0 } else { 0.42 })
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |style| if enabled { style.bg(colors.hover) } else { style })
        .child(app_icon(icon, 14., if enabled { colors.text } else { colors.muted }))
}

fn create_table_popover_text_button(label: impl Into<SharedString>, colors: UiColors) -> Div {
    let label = label.into();
    div()
        .h(px(28.))
        .min_w(px(58.))
        .px_3()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(label)
}

fn create_table_index_fields_summary(index: &CreateTableIndex) -> String {
    index
        .columns
        .iter()
        .filter(|column| !column.name.trim().is_empty())
        .map(|column| {
            let mut part = fluxdb_core::MySqlDialect.quote_identifier(column.name.trim());
            if !column.sub_part.trim().is_empty() {
                part.push('(');
                part.push_str(column.sub_part.trim());
                part.push(')');
            }
            if !column.sort_order.trim().is_empty() {
                part.push(' ');
                if column.sort_order.trim().eq_ignore_ascii_case("DESC") {
                    part.push_str("DESC");
                } else {
                    part.push_str("ASC");
                }
            }
            part
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn create_table_labeled_input(
    label: impl Into<SharedString>,
    input: Entity<InputState>,
    width: f32,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label = label.into();
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(38.))
                .text_size(ui_text_size(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(label),
        )
        .child(create_table_input_box(input, width, window, colors, cx))
}

fn create_table_input_cell<T: 'static>(
    input: Entity<InputState>,
    width: f32,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<T>,
) -> Div {
    create_table_input_cell_enabled(input, width, true, window, colors, cx)
}

fn create_table_input_cell_enabled<T: 'static>(
    input: Entity<InputState>,
    width: f32,
    enabled: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<T>,
) -> Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_2()
        .flex()
        .items_center()
        .child(create_table_input_box_enabled(
            input,
            width - 18.,
            enabled,
            window,
            colors,
            cx,
        ))
}

fn create_table_input_box<T: 'static>(
    input: Entity<InputState>,
    width: f32,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<T>,
) -> Div {
    create_table_input_box_enabled(input, width, true, window, colors, cx)
}

fn create_table_input_box_enabled<T: 'static>(
    input: Entity<InputState>,
    width: f32,
    enabled: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<T>,
) -> Div {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    div()
        .w(px(width))
        .h(px(CREATE_TABLE_INPUT_HEIGHT))
        .rounded(colors.radius)
        .border_1()
        .border_color(create_table_input_border_color(focused && enabled, colors))
        .bg(if enabled { colors.input_bg } else { colors.panel_alt })
        .opacity(if enabled { 1.0 } else { 0.55 })
        .overflow_hidden()
        .flex()
        .items_center()
        .hover(move |style| {
            if enabled {
                style.border_color(create_table_input_hover_border_color(focused, colors))
            } else {
                style
            }
        })
        .child(
            Input::new(&input)
                .appearance(false)
                .focus_bordered(false)
                .disabled(!enabled)
                .w_full()
                .h_full()
                .px_2()
                .text_size(ui_text_size(13.)),
        )
}

#[derive(Clone)]
struct CreateTableCommentEditorResizeDrag;

impl Render for CreateTableCommentEditorResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

fn create_table_comment_cell<T: 'static>(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    column_id: u64,
    input: Entity<InputState>,
    editor_input: Entity<InputState>,
    editor_size: (f32, f32),
    width: f32,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<T>,
) -> Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_2()
        .flex()
        .items_center()
        .gap_1()
        .child(create_table_input_box(
            input,
            width - 48.,
            window,
            colors,
            cx,
        ))
        .child(create_table_comment_popover(
            view,
            tab_id,
            column_id,
            editor_input,
            editor_size,
            colors,
        ))
}

fn create_table_comment_popover(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    column_id: u64,
    editor_input: Entity<InputState>,
    editor_size: (f32, f32),
    colors: UiColors,
) -> Div {
    div().child(
        Popover::new((
            gpui::ElementId::Name(format!("create-table-comment-popover-{}", tab_id.0).into()),
            column_id.to_string(),
        ))
        .appearance(false)
        .anchor(Anchor::TopRight)
        .trigger(
            Button::new((
                gpui::ElementId::Name(format!("create-table-comment-trigger-{}", tab_id.0).into()),
                column_id.to_string(),
            ))
            .ghost()
            .xsmall()
            .w(px(26.))
            .h(px(CREATE_TABLE_INPUT_HEIGHT))
            .p_0()
            .child(app_icon(AppIcon::Maximize, 14., colors.muted)),
        )
        .content(move |_, window, cx| {
            let width = clamp_create_table_comment_editor_width(editor_size.0, window);
            let height = clamp_create_table_comment_editor_height(editor_size.1, window);
            let focused = editor_input.read(cx).focus_handle(cx).is_focused(window);

            div()
                .relative()
                .w(px(width))
                .h(px(height))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow_md()
                .p_3()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex_none()
                        .text_size(ui_text_size(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(tr("app.create_table.popover.edit_comment")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(create_table_input_border_color(focused, colors))
                        .bg(colors.input_bg)
                        .overflow_hidden()
                        .hover(move |style| {
                            style.border_color(create_table_input_hover_border_color(
                                focused, colors,
                            ))
                        })
                        .child(
                            Input::new(&editor_input)
                                .appearance(false)
                                .focus_bordered(false)
                                .w_full()
                                .h_full()
                                .px_2()
                                .text_size(ui_text_size(13.)),
                        ),
                )
                .child(create_table_comment_editor_resize_handle(
                    view.clone(),
                    tab_id,
                    column_id,
                    width,
                    height,
                    colors,
                    cx,
                ))
        }),
    )
}

fn create_table_comment_editor_resize_handle<T: 'static>(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    column_id: u64,
    width: f32,
    height: f32,
    colors: UiColors,
    cx: &mut Context<T>,
) -> impl IntoElement {
    let mouse_down_view = view.clone();
    let drag_move_view = view.clone();
    let mouse_up_view = view;
    div()
        .id(gpui::ElementId::Name(
            format!(
                "create-table-comment-editor-resize-{}-{}",
                tab_id.0, column_id
            )
            .into(),
        ))
        .absolute()
        .right(px(5.))
        .bottom(px(5.))
        .size(px(16.))
        .cursor_nwse_resize()
        .rounded(colors.radius * 0.5)
        .opacity(0.7)
        .hover(move |style| style.bg(colors.hover).opacity(1.0))
        .child(
            div()
                .absolute()
                .right(px(3.))
                .bottom(px(3.))
                .w(px(7.))
                .h(px(7.))
                .border_r_1()
                .border_b_1()
                .border_color(colors.muted),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_, event: &MouseDownEvent, _, cx| {
                let _ = mouse_down_view.update(cx, |this, cx| {
                    this.create_table_comment_editor_resize_start =
                        Some(CreateTableCommentEditorResizeStart {
                            tab_id,
                            column_id,
                            x: f32::from(event.position.x),
                            y: f32::from(event.position.y),
                            width,
                            height,
                        });
                    cx.notify();
                });
                cx.stop_propagation();
            }),
        )
        .on_drag(CreateTableCommentEditorResizeDrag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_drag_move(cx.listener(
            move |_, event: &DragMoveEvent<CreateTableCommentEditorResizeDrag>, window, cx| {
                let _ = drag_move_view.update(cx, |this, cx| {
                    let Some(start) = this.create_table_comment_editor_resize_start else {
                        return;
                    };
                    if start.tab_id != tab_id || start.column_id != column_id {
                        return;
                    }
                    let next_width = clamp_create_table_comment_editor_width(
                        start.width + f32::from(event.event.position.x) - start.x,
                        window,
                    );
                    let next_height = clamp_create_table_comment_editor_height(
                        start.height + f32::from(event.event.position.y) - start.y,
                        window,
                    );
                    this.create_table_comment_editor_sizes
                        .insert((tab_id, column_id), (next_width, next_height));
                    cx.notify();
                });
                cx.stop_propagation();
            },
        ))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |_, _, _, cx| {
                let _ = mouse_up_view.update(cx, |this, cx| {
                    this.create_table_comment_editor_resize_start = None;
                    cx.notify();
                });
                cx.stop_propagation();
            }),
        )
}

fn clamp_create_table_comment_editor_width(width: f32, window: &Window) -> f32 {
    let max_width = (f32::from(window.viewport_size().width) - 64.)
        .max(CREATE_TABLE_COMMENT_EDITOR_MIN_WIDTH);
    width.clamp(CREATE_TABLE_COMMENT_EDITOR_MIN_WIDTH, max_width)
}

fn clamp_create_table_comment_editor_height(height: f32, window: &Window) -> f32 {
    let max_height = (f32::from(window.viewport_size().height) - 96.)
        .max(CREATE_TABLE_COMMENT_EDITOR_MIN_HEIGHT);
    height.clamp(CREATE_TABLE_COMMENT_EDITOR_MIN_HEIGHT, max_height)
}

#[derive(Clone, Copy)]
enum CreateTableColumnAction {
    MoveUp,
    MoveDown,
    Remove,
}


fn create_table_operations_cell(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    column_id: u64,
    can_move_up: bool,
    can_move_down: bool,
    colors: UiColors,
) -> Div {
    div()
        .w(px(CREATE_TABLE_OPERATIONS_WIDTH))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_2()
        .flex()
        .items_center()
        .gap_1()
        .child(create_table_operation_button(
            tab_id,
            column_id,
            AppIcon::ChevronUp,
            None,
            can_move_up,
            CreateTableColumnAction::MoveUp,
            view.clone(),
            colors,
        ))
        .child(create_table_operation_button(
            tab_id,
            column_id,
            AppIcon::ChevronDown,
            None,
            can_move_down,
            CreateTableColumnAction::MoveDown,
            view.clone(),
            colors,
        ))
        .child(create_table_operation_button(
            tab_id,
            column_id,
            AppIcon::Close,
            Some(tr("app.create_table.remove")),
            true,
            CreateTableColumnAction::Remove,
            view,
            colors,
        ))
}


fn create_table_operation_button(
    tab_id: TabId,
    column_id: u64,
    icon: AppIcon,
    label: Option<SharedString>,
    enabled: bool,
    action: CreateTableColumnAction,
    view: WeakEntity<NavicatMain>,
    colors: UiColors,
) -> Div {
    let color = if enabled { colors.text } else { colors.muted };
    let button = div()
        .h(px(26.))
        .min_w(px(if label.is_some() { 62. } else { 28. }))
        .px_2()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .opacity(if enabled { 1.0 } else { 0.42 })
        .hover(move |style| if enabled { style.bg(colors.hover) } else { style })
        .child(app_icon(icon, 15., color))
        .when_some(label, |this, label| {
            this.child(
                div()
                    .text_size(ui_text_size(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(label),
            )
        });

    if !enabled {
        return button;
    }

    button.cursor_pointer().on_mouse_down(
        MouseButton::Left,
        move |_, _, cx| {
            let command = match action {
                CreateTableColumnAction::MoveUp => AppCommand::MoveCreateTableColumnUp {
                    tab_id,
                    column_id,
                },
                CreateTableColumnAction::MoveDown => AppCommand::MoveCreateTableColumnDown {
                    tab_id,
                    column_id,
                },
                CreateTableColumnAction::Remove => AppCommand::RemoveCreateTableColumn {
                    tab_id,
                    column_id,
                },
            };
            let _ = view.update(cx, |this, cx| {
                this.dispatch(command, cx);
            });
            cx.stop_propagation();
        },
    )
}

fn create_table_type_select_cell(
    select: Entity<SelectState<SearchableVec<CreateTableTypeOption>>>,
    input: Entity<InputState>,
    width: f32,
    row_index: usize,
    editing: bool,
    tab_id: TabId,
    column_id: u64,
    model_value: &str,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let cancel_input = input.clone();
    let cancel_select = select.clone();
    let model_value = model_value.to_string();
    div()
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_2()
        .flex()
        .items_center()
        .capture_action(cx.listener(
            move |this, _: &gpui_component::input::Escape, window, cx| {
                if this.create_table_custom_type_editing != Some((tab_id, column_id)) {
                    cx.propagate();
                    return;
                }
                this.create_table_custom_type_editing = None;
                cancel_input.update(cx, |input, cx| {
                    input.set_value(model_value.clone(), window, cx);
                });
                cancel_select
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
                cx.stop_propagation();
                cx.notify();
            },
        ))
        .child(if editing {
            if !input.read(cx).focus_handle(cx).is_focused(window) {
                input.update(cx, |input, cx| {
                    input.focus(window, cx);
                    input.select_all(window, cx);
                });
            }
            div()
                .debug_selector(move || format!("create-table-custom-type-input-{row_index}"))
                .child(create_table_input_box(
                    input,
                    width - 18.,
                    window,
                    colors,
                    cx,
                ))
        } else {
            div()
                .w(px(width - 18.))
                .h(px(CREATE_TABLE_INPUT_HEIGHT))
                .debug_selector(move || format!("create-table-type-select-{row_index}"))
                .child(
                    Select::new(&select)
                        .small()
                        .text_size(ui_text_size(13.))
                        .placeholder("varchar")
                        .search_placeholder(
                            tr("app.create_table.search_placeholder_select_or_type"),
                        )
                        .size_full()
                        .menu_width(px(220.)),
                )
        })
}

fn create_table_select_cell_with_placeholder<T: 'static, I: SelectItem + 'static>(
    select: Entity<SelectState<SearchableVec<I>>>,
    width: f32,
    placeholder: impl Into<SharedString>,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<T>,
) -> Div {
    create_table_select_cell_with_placeholder_enabled(
        select,
        width,
        placeholder,
        true,
        window,
        colors,
        cx,
    )
}

fn create_table_select_cell_with_placeholder_enabled<T: 'static, I: SelectItem + 'static>(
    select: Entity<SelectState<SearchableVec<I>>>,
    width: f32,
    placeholder: impl Into<SharedString>,
    enabled: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<T>,
) -> Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_2()
        .flex()
        .items_center()
        .child(create_table_type_select_box(
            select,
            width - 18.,
            placeholder,
            enabled,
            false,
            window,
            colors,
            cx,
        ))
}

fn create_table_select_cell_with_loading<T: 'static, I: SelectItem + 'static>(
    select: Entity<SelectState<SearchableVec<I>>>,
    width: f32,
    placeholder: impl Into<SharedString>,
    loading: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<T>,
) -> Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_2()
        .flex()
        .items_center()
        .child(create_table_type_select_box(
            select,
            width - 18.,
            placeholder,
            true,
            loading,
            window,
            colors,
            cx,
        ))
}

fn create_table_type_select_box<T: 'static, I: SelectItem + 'static>(
    select: Entity<SelectState<SearchableVec<I>>>,
    width: f32,
    placeholder: impl Into<SharedString>,
    enabled: bool,
    loading: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<T>,
) -> Div {
    let focused = select.read(cx).focus_handle(cx).is_focused(window);
    let interactive = enabled && !loading;
    div()
        .w(px(width))
        .h(px(CREATE_TABLE_INPUT_HEIGHT))
        .rounded(colors.radius)
        .border_1()
        .border_color(create_table_input_border_color(focused, colors))
        .bg(colors.input_bg)
        .opacity(if interactive { 1.0 } else { 0.52 })
        .overflow_hidden()
        .flex()
        .items_center()
        .hover(move |style| {
            if interactive {
                style.border_color(create_table_input_hover_border_color(focused, colors))
            } else {
                style
            }
        })
        .when(loading, |this| {
            this.child(
                div()
                    .pl_2()
                    .flex()
                    .items_center()
                    .child(loading_spinner_with_color(13., colors.muted)),
            )
        })
        .child(
            Select::new(&select)
                .appearance(false)
                .small()
                .disabled(!interactive)
                .placeholder(placeholder)
                .search_placeholder(tr("app.create_table.search_placeholder_select_or_type"))
                .w_full()
                .h_full()
                .menu_width(px(220.)),
        )
}

fn create_table_type_options(database_kind: DatabaseKind) -> Vec<CreateTableTypeOption> {
    create_table_common_type_options(database_kind)
        .into_iter()
        .chain(create_table_special_type_options(database_kind))
        .map(|value| CreateTableTypeOption {
            value,
            custom_entry: false,
        })
        .chain(std::iter::once(CreateTableTypeOption {
            value: String::new(),
            custom_entry: true,
        }))
        .collect()
}

/// 当前类型不在预设选项时补到末尾并选中，避免已有列类型显示成灰色占位文字。
fn create_table_type_select_items(
    database_kind: DatabaseKind,
    value: &str,
) -> (Vec<CreateTableTypeOption>, Option<IndexPath>) {
    let value = value.trim();
    let mut options = create_table_type_options(database_kind);
    if !value.is_empty()
        && !options
            .iter()
            .any(|option| !option.custom_entry && option.value.eq_ignore_ascii_case(value))
    {
        options.insert(
            options.len() - 1,
            CreateTableTypeOption {
                value: value.to_string(),
                custom_entry: false,
            },
        );
    }
    let index = options
        .iter()
        .position(|option| !option.custom_entry && option.value.eq_ignore_ascii_case(value))
        .map(IndexPath::new);
    (options, index)
}

fn create_table_default_cell<T: 'static>(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    column_id: u64,
    input: Entity<InputState>,
    value: &str,
    width: f32,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<T>,
) -> Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_2()
        .flex()
        .items_center()
        .child(create_table_default_input_box(
            view,
            tab_id,
            column_id,
            input,
            value,
            width - 18.,
            window,
            colors,
            cx,
        ))
}

fn create_table_default_input_box<T: 'static>(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    column_id: u64,
    input: Entity<InputState>,
    value: &str,
    width: f32,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<T>,
) -> Div {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    let input_width = width - 24.;
    div()
        .w(px(width))
        .h(px(CREATE_TABLE_INPUT_HEIGHT))
        .rounded(colors.radius)
        .border_1()
        .border_color(create_table_input_border_color(focused, colors))
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .hover(move |style| style.border_color(create_table_input_hover_border_color(focused, colors)))
        .child(
            Input::new(&input)
                .appearance(false)
                .focus_bordered(false)
                .w(px(input_width))
                .h_full()
                .px_2()
                .text_size(ui_text_size(13.)),
        )
        .child(create_table_default_popover(
            view,
            tab_id,
            column_id,
            input,
            value.to_string(),
            width,
            colors,
        ))
}

fn create_table_default_popover(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    column_id: u64,
    input: Entity<InputState>,
    value: String,
    width: f32,
    colors: UiColors,
) -> Div {
    div()
        .w(px(24.))
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .justify_center()
        .child(
            Popover::new((
                gpui::ElementId::Name(format!("create-table-default-popover-{}", tab_id.0).into()),
                column_id.to_string(),
            ))
                .appearance(false)
                .anchor(Anchor::TopRight)
                .trigger(
                    Button::new((
                        gpui::ElementId::Name(
                            format!("create-table-default-trigger-{}", tab_id.0).into(),
                        ),
                        column_id.to_string(),
                    ))
                        .ghost()
                        .xsmall()
                        .w(px(24.))
                        .h_full()
                        .p_0()
                        .child(app_icon(AppIcon::ChevronDown, 13., colors.muted)),
                )
                .content(move |_, _, cx| {
                    let popover = cx.entity();
                    let mut menu = div()
                        .w(px(width))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.panel_bg)
                        .shadow_md()
                        .overflow_hidden();
                    for (label, default_value) in [
                        (tr("app.create_table.index_default_value.none"), ""),
                        (tr("app.create_table.index_default_value.null"), "NULL"),
                        (
                            tr("app.create_table.index_default_value.empty_string"),
                            "''",
                        ),
                    ] {
                        menu = menu.child(create_table_default_menu_item(
                            view.clone(),
                            popover.clone(),
                            tab_id,
                            column_id,
                            input.clone(),
                            label,
                            default_value,
                            value.trim() == default_value,
                            colors,
                        ));
                    }
                    menu
                }),
        )
}

fn create_table_default_menu_item(
    view: WeakEntity<NavicatMain>,
    popover: Entity<gpui_component::popover::PopoverState>,
    tab_id: TabId,
    column_id: u64,
    input: Entity<InputState>,
    label: SharedString,
    value: &'static str,
    active: bool,
    colors: UiColors,
) -> Div {
    div()
        .h(px(28.))
        .px_2()
        .flex()
        .items_center()
        .text_size(ui_text_size(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if active { colors.text } else { colors.muted })
        .bg(if active { colors.hover } else { colors.panel_bg })
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
        .on_mouse_down(
            MouseButton::Left,
            move |_, window, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.set_create_table_column_input(
                        tab_id,
                        column_id,
                        CreateTableColumnField::DefaultValue,
                        value.to_string(),
                        cx,
                    );
                    input.update(cx, |input, cx| {
                        input.set_value(value.to_string(), window, cx);
                    });
                });
                popover.update(cx, |state, cx| {
                    state.dismiss(window, cx);
                });
                cx.stop_propagation();
            },
        )
        .child(label)
}

fn create_table_flag_cell(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    column_id: u64,
    label: impl Into<SharedString>,
    checked: bool,
    flag: CreateTableColumnFlag,
    width: f32,
    colors: UiColors,
) -> Div {
    let label = label.into();
    div()
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            move |_, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.dispatch(
                        AppCommand::ToggleCreateTableColumnFlag {
                            tab_id,
                            column_id,
                            flag,
                        },
                        cx,
                    );
                });
                cx.stop_propagation();
            },
        )
        .child(Checkbox::new(create_table_checkbox_id(tab_id, column_id, flag)).checked(checked))
        .child(label)
}

fn create_table_column_options(
    tab_id: TabId,
    database_kind: DatabaseKind,
    column: Option<&CreateTableColumn>,
    original_column: Option<&CreateTableColumn>,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let Some(column) = column else {
        return div()
            .flex_none()
            .h(px(86.))
            .border_t_1()
            .border_color(colors.border)
            .flex()
            .items_center()
            .px_3()
            .text_size(ui_text_size(13.))
            .text_color(colors.muted)
            .child(tr("app.create_table.error.select_field"))
            .into_any_element();
    };
    let id = column.id;
    let data_type = column.data_type.to_lowercase();
    let capabilities = create_table_provider(database_kind).type_capabilities(&data_type);
    let is_enum = matches!(database_kind, DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb)
        && matches!(create_table_type_base(&column.data_type).as_str(), "enum" | "set");
    let enum_max_height = f32::from(window.viewport_size().height) * 0.3;
    let enum_panel_height =
        create_table_enum_options_height(column, original_column).min(enum_max_height);
    let mut options = div()
        .w_full()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_3()
        .when(is_enum, |this| this.h_full().flex_col());

    if is_enum {
        options = options.child(create_table_enum_values_editor(
            tab_id,
            column,
            original_column,
            this,
            window,
            colors,
            cx,
        ));
    }

    if !create_table_type_is_known(database_kind, &column.data_type) {
        let value = column.data_type.clone();
        let column_id = column.id;
        options = options.child(
            div()
                .debug_selector(move || format!("create-table-custom-type-summary-{column_id}"))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(84.))
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(tr("app.create_table.column.custom_type")),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .max_w(px(360.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(ui_text_size(12.))
                        .text_color(colors.text)
                        .child(value),
                )
                .child(
                    Button::new(format!("custom-type-edit-{tab_id:?}-{column_id}"))
                        .ghost()
                        .xsmall()
                        .debug_selector(move || {
                            format!("create-table-custom-type-edit-{column_id}")
                        })
                        .child(tr("app.create_table.column.custom_type_edit"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.create_table_custom_type_editing = Some((tab_id, column_id));
                            cx.notify();
                        })),
                ),
        );
    }

    if capabilities.text_options {
        let charset_select = this.create_table_select(
            CreateTableSelectKey::ColumnCharset(tab_id, id),
            create_table_inherit_select_options(create_database_charset_options()),
            &column.charset,
            window,
            cx,
        );
        let collation_charset = if column.charset.trim().is_empty() {
            "utf8mb4"
        } else {
            column.charset.trim()
        };
        let collation_select = this.create_table_select(
            CreateTableSelectKey::ColumnCollation(tab_id, id),
            create_table_inherit_select_options(create_database_collation_options(collation_charset)),
            &column.collation,
            window,
            cx,
        );
        options = options
            .child(create_table_option_select(
                tr("app.create_table.column.charset"),
                charset_select,
                180.,
                tr("app.create_table.column.charset"),
                window,
                colors,
                cx,
            ))
            .child(create_table_option_select(
                tr("app.create_table.column.collation"),
                collation_select,
                220.,
                tr("app.create_table.column.collation"),
                window,
                colors,
                cx,
            ))
            .child(create_table_option_flag(
                tab_id,
                id,
                tr("app.create_table.column.binary"),
                column.binary,
                CreateTableColumnFlag::Binary,
                colors,
                cx,
            ));
    }

    if column.primary_key && capabilities.key_length {
        options = options.child(create_table_option_input(
            tr("app.create_table.column.prefix_length"),
            this.create_table_input(
                CreateTableInputKey::ColumnKeyLength(tab_id, id),
                tr("app.create_table.column.prefix_length"),
                &column.key_length,
                window,
                cx,
            ),
            120.,
            window,
            colors,
            cx,
        ));
    }

    if (capabilities.number_options || capabilities.auto_increment) && capabilities.scale {
        options = options.child(create_table_option_input(
            tr("app.create_table.column.decimal"),
            this.create_table_input(
                CreateTableInputKey::ColumnScale(tab_id, id),
                tr("app.create_table.column.decimal"),
                &column.scale,
                window,
                cx,
            ),
            120.,
            window,
            colors,
            cx,
        ));
    }

    if capabilities.unsigned {
        options = options.child(create_table_option_flag(
            tab_id,
            id,
            tr("app.create_table.column.unsigned"),
            column.unsigned,
            CreateTableColumnFlag::Unsigned,
            colors,
            cx,
        ));
    }
    if capabilities.zerofill {
        options = options.child(create_table_option_flag(
            tab_id,
            id,
            tr("app.create_table.column.zero_fill"),
            column.zerofill,
            CreateTableColumnFlag::Zerofill,
            colors,
            cx,
        ));
    }
    if capabilities.auto_increment {
        options = options.child(create_table_option_flag(
            tab_id,
            id,
            tr("app.create_table.column.auto_increment"),
            column.auto_increment,
            CreateTableColumnFlag::AutoIncrement,
            colors,
            cx,
        ));
    }

    if capabilities.auto_update_time {
        options = options.child(create_table_option_flag(
            tab_id,
            id,
            tr("app.create_table.column.auto_update_timestamp"),
            column.auto_update_time,
            CreateTableColumnFlag::AutoUpdateTime,
            colors,
            cx,
        ));
    }

    let options_scroll = if is_enum {
        div()
            .flex_1()
            .min_h(px(0.))
            .px_3()
            .py_2()
            .child(options)
            .into_any_element()
    } else {
        div()
            .debug_selector(|| "create-table-column-options-scroll".into())
            .flex_1()
            .overflow_hidden()
            .child(
                div().flex_1().overflow_hidden().child(
                    div()
                        .size_full()
                        .overflow_y_scrollbar()
                        .px_3()
                        .py_2()
                        .child(options),
                ),
            )
            .into_any_element()
    };

    div()
        .flex_none()
        .min_h(px(74.))
        .debug_selector(|| "create-table-column-options".into())
        .when(!is_enum, |this| this.max_h(gpui::relative(0.3)))
        .when(is_enum, |this| this.h(px(enum_panel_height)).max_h(px(enum_max_height)))
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(options_scroll)
        .into_any_element()
}

fn create_table_enum_options_height(
    column: &CreateTableColumn,
    original: Option<&CreateTableColumn>,
) -> f32 {
    let rows = column.enum_values.len();
    let rows_height = rows as f32 * 28.;
    let row_gaps = rows.saturating_sub(1) as f32 * 4.;
    let warning_height = if create_table_enum_values_are_risky(column, original) {
        42. + 8.
    } else {
        0.
    };
    // 头部、头部到成员列表的间距、成员行与面板上下 padding。
    24. + 8. + rows_height + row_gaps + warning_height + 16.
}

fn create_table_enum_values_are_risky(
    column: &CreateTableColumn,
    original: Option<&CreateTableColumn>,
) -> bool {
    let Some(original) = original else {
        return false;
    };
    let removed_existing = original
        .enum_values
        .iter()
        .any(|value| !column.enum_values.contains(value));
    let mut old = original.enum_values.clone();
    let mut new = column.enum_values.clone();
    old.sort();
    new.sort();
    removed_existing || (old == new && original.enum_values != column.enum_values)
}

fn create_table_enum_values_editor(
    tab_id: TabId,
    column: &CreateTableColumn,
    original: Option<&CreateTableColumn>,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let column_id = column.id;
    let base = create_table_type_base(&column.data_type);
    let risky = create_table_enum_values_are_risky(column, original);
    let scroll = this.create_table_enum_scroll(tab_id, column_id);
    let new_member_index = column.enum_values.len();
    let add_button = Button::new(format!("enum-add-{tab_id:?}-{column_id}"))
        .ghost()
        .xsmall()
        .debug_selector(move || format!("create-table-enum-add-{column_id}"))
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(app_icon(AppIcon::Plus, 13., colors.muted))
                .child(tr("app.create_table.column.enum_add_value")),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            let scroll = this.create_table_enum_scroll(tab_id, column_id);
            this.dispatch(
                AppCommand::AddCreateTableColumnEnumValue { tab_id, column_id },
                cx,
            );
            scroll.scroll_to_top_of_item(new_member_index);
        }));
    let mut rows = Vec::new();

    for (index, value) in column.enum_values.iter().enumerate() {
        let input = this.create_table_input(
            CreateTableInputKey::ColumnEnumValue(tab_id, column_id, index),
            tr("app.create_table.column.enum_value_placeholder"),
            value,
            window,
            cx,
        );
        rows.push(
            div()
                .debug_selector(move || {
                    format!("create-table-enum-value-row-{column_id}-{index}")
                })
                .h(px(28.))
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .debug_selector(move || {
                            format!("create-table-enum-value-input-{column_id}-{index}")
                        })
                        .child(create_table_input_box(input, 260., window, colors, cx)),
                )
                .child(create_table_enum_value_action(
                    format!("enum-up-{tab_id:?}-{column_id}-{index}"),
                    AppIcon::ChevronUp,
                    tr("app.create_table.column.enum_move_up"),
                    index > 0,
                    AppCommand::MoveCreateTableColumnEnumValueUp {
                        tab_id,
                        column_id,
                        index,
                    },
                    colors,
                    cx,
                ))
                .child(create_table_enum_value_action(
                    format!("enum-down-{tab_id:?}-{column_id}-{index}"),
                    AppIcon::ChevronDown,
                    tr("app.create_table.column.enum_move_down"),
                    index + 1 < column.enum_values.len(),
                    AppCommand::MoveCreateTableColumnEnumValueDown {
                        tab_id,
                        column_id,
                        index,
                    },
                    colors,
                    cx,
                ))
                .child(create_table_enum_value_action(
                    format!("enum-remove-{tab_id:?}-{column_id}-{index}"),
                    AppIcon::Trash,
                    tr("app.create_table.column.enum_remove_value"),
                    true,
                    AppCommand::RemoveCreateTableColumnEnumValue {
                        tab_id,
                        column_id,
                        index,
                    },
                    colors,
                    cx,
                )),
        );
    }

    let header = div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .text_size(ui_text_size(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(if base == "set" {
                    tr("app.create_table.column.set_values")
                } else {
                    tr("app.create_table.column.enum_values")
                }),
        )
        .child(
            div()
                .text_size(ui_text_size(12.))
                .text_color(colors.muted)
                .child(format!(
                    "{} / {}",
                    column.enum_values.len(),
                    if base == "set" { 64 } else { 65535 }
                )),
        )
        .child(div().flex_1())
        .child(add_button);

    let mut member_list = div()
        .id(gpui::ElementId::Name(
            format!("create-table-enum-members-{}-{column_id}", tab_id.0).into(),
        ))
        .debug_selector(|| "create-table-enum-members-scroll".into())
        .size_full()
        .flex()
        .flex_col()
        .gap_1()
        .track_scroll(&scroll)
        .overflow_y_scroll();
    member_list = member_list.children(rows);

    div()
        .debug_selector(move || format!("create-table-enum-editor-{column_id}"))
        .w_full()
        .h_full()
        .flex()
        .flex_col()
        .gap_2()
        .child(header)
        .child(
            div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .child(
                    member_list,
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(Scrollbar::new(&scroll)),
                ),
        )
        .when(risky, |this| {
            this.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(colors.radius)
                    .bg(colors.panel_bg)
                    .border_1()
                    .border_color(colors.warning)
                    .text_size(ui_text_size(12.))
                    .text_color(colors.warning)
                    .child(app_icon(AppIcon::TriangleAlert, 14., colors.warning))
                    .child(tr("app.create_table.column.enum_values_risk")),
            )
        })
}

fn create_table_enum_value_action(
    id: String,
    icon: AppIcon,
    tooltip: SharedString,
    enabled: bool,
    command: AppCommand,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Button {
    Button::new(id.clone())
        .ghost()
        .xsmall()
        .w(px(24.))
        .h(px(24.))
        .p_0()
        .debug_selector(move || id)
        .disabled(!enabled)
        .tooltip(tooltip)
        .child(app_icon(icon, 13., colors.text))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.dispatch(command.clone(), cx);
            cx.stop_propagation();
        }))
}

fn create_table_option_input(
    label: impl Into<SharedString>,
    input: Entity<InputState>,
    width: f32,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label = label.into();
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .w(px(56.))
                .text_size(ui_text_size(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(label),
        )
        .child(create_table_input_box(input, width, window, colors, cx))
}

fn create_table_option_select(
    label: impl Into<SharedString>,
    select: Entity<SelectState<SearchableVec<CreateTableSelectOption>>>,
    width: f32,
    placeholder: impl Into<SharedString>,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label = label.into();
    let placeholder = placeholder.into();
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .w(px(56.))
                .text_size(ui_text_size(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(label),
        )
        .child(create_table_option_select_box(
            select,
            width,
            placeholder,
            window,
            colors,
            cx,
        ))
}

fn create_table_option_select_box<T: 'static, I: SelectItem + 'static>(
    select: Entity<SelectState<SearchableVec<I>>>,
    width: f32,
    placeholder: impl Into<SharedString>,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<T>,
) -> Div {
    let placeholder = placeholder.into();
    let focused = select.read(cx).focus_handle(cx).is_focused(window);
    div()
        .w(px(width))
        .h(px(CREATE_TABLE_INPUT_HEIGHT))
        .rounded(colors.radius)
        .border_1()
        .border_color(create_table_input_border_color(focused, colors))
        .bg(colors.input_bg)
        .overflow_hidden()
        .flex()
        .items_center()
        .hover(move |style| style.border_color(create_table_input_hover_border_color(focused, colors)))
        .child(
            Select::new(&select)
                .appearance(false)
                .small()
                .placeholder(placeholder)
                .search_placeholder(tr("app.create_table.search_placeholder_select"))
                .w_full()
                .h_full()
                .menu_width(px(width.max(220.))),
        )
}

fn create_table_option_flag(
    tab_id: TabId,
    column_id: u64,
    label: impl Into<SharedString>,
    checked: bool,
    flag: CreateTableColumnFlag,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label = label.into();
    div()
        .h(px(26.))
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(
                    AppCommand::ToggleCreateTableColumnFlag {
                        tab_id,
                        column_id,
                        flag,
                    },
                    cx,
                );
                cx.stop_propagation();
            }),
        )
        .child(Checkbox::new(create_table_checkbox_id(tab_id, column_id, flag)).checked(checked))
        .child(
            div()
                .text_size(ui_text_size(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(label),
        )
}

fn create_table_supports_length(database_kind: DatabaseKind, data_type: &str) -> bool {
    let capabilities = create_table_provider(database_kind).type_capabilities(data_type);
    capabilities.length || capabilities.bit_length
}

fn create_table_length_placeholder(database_kind: DatabaseKind, data_type: &str) -> &'static str {
    create_table_provider(database_kind)
        .default_length(data_type)
        .unwrap_or("")
}

fn create_table_type_is_known(database_kind: DatabaseKind, data_type: &str) -> bool {
    let data_type = data_type.trim();
    if data_type.is_empty() {
        return true;
    }
    let base = data_type
        .split_once('(')
        .map_or(data_type, |(base, _)| base.trim());
    create_table_provider(database_kind)
        .type_options()
        .iter()
        .any(|option| {
            option.eq_ignore_ascii_case(data_type)
                || (option.eq_ignore_ascii_case(base) && !data_type.contains(['(', '[', '.']))
        })
}

fn create_table_type_base(data_type: &str) -> String {
    data_type
        .trim()
        .split_once('(')
        .map_or(data_type.trim(), |(base, _)| base.trim())
        .to_ascii_lowercase()
}

fn create_table_inherit_select_options(mut options: Vec<String>) -> Vec<String> {
    if !options.iter().any(String::is_empty) {
        options.insert(0, String::new());
    }
    options
}

fn create_table_select_index(options: &[String], value: &str) -> Option<IndexPath> {
    let value = value.trim();
    options
        .iter()
        .position(|item| item.eq_ignore_ascii_case(value))
        .map(IndexPath::new)
}

fn create_table_table_cell(text: impl Into<String>, colors: UiColors) -> Div {
    div()
        .size_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .overflow_hidden()
        .text_size(ui_text_size(13.))
        .text_color(colors.text)
        .child(text.into())
}

fn create_table_flag_id(flag: CreateTableColumnFlag) -> &'static str {
    match flag {
        CreateTableColumnFlag::Nullable => "create-table-nullable",
        CreateTableColumnFlag::PrimaryKey => "create-table-primary-key",
        CreateTableColumnFlag::AutoIncrement => "create-table-auto-increment",
        CreateTableColumnFlag::AutoUpdateTime => "create-table-auto-update-time",
        CreateTableColumnFlag::Unsigned => "create-table-unsigned",
        CreateTableColumnFlag::Zerofill => "create-table-zerofill",
        CreateTableColumnFlag::Binary => "create-table-binary",
    }
}

fn create_table_checkbox_id(
    tab_id: TabId,
    column_id: u64,
    flag: CreateTableColumnFlag,
) -> gpui::ElementId {
    gpui::ElementId::Name(
        format!("{}-{}-{column_id}", create_table_flag_id(flag), tab_id.0).into(),
    )
}

fn create_table_empty_state(message: impl Into<SharedString>, colors: UiColors) -> Div {
    let message: SharedString = message.into();
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .items_center()
        .justify_center()
        .text_size(ui_text_size(13.))
        .text_color(colors.muted)
        .child(message)
}

fn create_table_sql_preview(
    tab_id: TabId,
    create: &CreateTableState,
    editor_theme: editor_component::EditorTheme,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let preview = create.sql_preview();
    let text = preview.clone().unwrap_or_else(|message| message.to_string());
    let editor = sql_preview_editor(
        SharedString::from(format!("create-table-sql-preview-{}", tab_id.0)),
        &text,
        create.database_kind,
        // 预览与 DDL 页签一致：不自动换行，长语句横向滚动查看。
        false,
        editor_theme,
        window,
        cx,
    );

    div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(42.))
                .px_3()
                .border_b_1()
                .border_color(colors.border)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(tr("app.create_table.tab.sql_preview")),
                )
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(if preview.is_ok() { "1" } else { "0" }),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .p_3()
                .child(editor),
        )
}

fn create_table_ddl_preview(
    tab_id: TabId,
    create: &CreateTableState,
    editor_theme: editor_component::EditorTheme,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let preview = create.ddl_preview();
    let text = preview.clone().unwrap_or_else(|message| message.to_string());
    let editor = sql_preview_editor(
        SharedString::from(format!("create-table-ddl-preview-{}", tab_id.0)),
        &text,
        create.database_kind,
        // 与 SQL 预览一致：不自动换行，长列定义横向滚动查看。
        false,
        editor_theme,
        window,
        cx,
    );

    // 结构与 SQL 预览保持一致：外层必须是 flex 列，编辑器才有一个高度确定的容器
    // （编辑器根元素 `size_full()` 且内部全是绝对定位，容器高度塌陷时预览就是一片空白）。
    div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(div().flex_1().min_h(px(0.)).p_3().child(editor))
}

fn create_table_input_border_color(focused: bool, colors: UiColors) -> gpui::Rgba {
    if focused {
        if colors.is_dark {
            rgb(0x8ab4ff)
        } else {
            rgb(0x111111)
        }
    } else {
        colors.border
    }
}

fn create_table_input_hover_border_color(focused: bool, colors: UiColors) -> gpui::Rgba {
    if focused {
        create_table_input_border_color(true, colors)
    } else if colors.is_dark {
        rgb(0x4a5260)
    } else {
        rgb(0x9aa4b2)
    }
}

#[cfg(test)]
impl NavicatMain {
    fn create_table_type_value_for_test(
        &self,
        tab_id: TabId,
        column_id: u64,
        cx: &App,
    ) -> Option<String> {
        self.create_table_type_selects
            .get(&(tab_id, column_id))
            .and_then(|select| select.read(cx).selected_value().cloned())
    }
}

#[cfg(test)]
mod create_table_type_select_tests {
    use super::*;

    #[test]
    fn custom_type_is_appended_and_selected() {
        let (options, index) = create_table_type_select_items(DatabaseKind::Postgres, "order_state");
        assert_eq!(
            options.get(options.len() - 2).map(|option| option.value.as_str()),
            Some("order_state")
        );
        assert!(options.last().is_some_and(|option| option.custom_entry));
        assert_eq!(index, Some(IndexPath::new(options.len() - 2)));
    }

    #[test]
    fn canonical_type_is_not_duplicated_by_case() {
        let (options, index) = create_table_type_select_items(DatabaseKind::MySql, "VARCHAR");
        assert_eq!(
            options
                .iter()
                .filter(|option| option.value.eq_ignore_ascii_case("varchar"))
                .count(),
            1
        );
        assert_eq!(
            index,
            options
                .iter()
                .position(|option| option.value.eq_ignore_ascii_case("varchar"))
                .map(IndexPath::new)
        );
    }

    #[test]
    fn special_types_are_grouped_after_common_types() {
        for kind in [DatabaseKind::MySql, DatabaseKind::Postgres] {
            let common = fluxdb_app::create_table_common_type_options(kind);
            let special = fluxdb_app::create_table_special_type_options(kind);
            let (options, _) = create_table_type_select_items(kind, "");
            assert_eq!(options.len(), common.len() + special.len() + 1, "{kind:?}");
            assert_eq!(
                options[..common.len()]
                    .iter()
                    .map(|option| option.value.as_str())
                    .collect::<Vec<_>>(),
                common.iter().map(String::as_str).collect::<Vec<_>>(),
                "{kind:?}"
            );
            assert_eq!(
                options[common.len()..common.len() + special.len()]
                    .iter()
                    .map(|option| option.value.as_str())
                    .collect::<Vec<_>>(),
                special.iter().map(String::as_str).collect::<Vec<_>>(),
                "{kind:?}"
            );
            assert!(options.last().is_some_and(|option| option.custom_entry));
        }
    }

    #[test]
    fn sqlite_common_types_use_structured_parameters() {
        for data_type in ["varchar", "decimal"] {
            assert!(
                create_table_type_is_known(DatabaseKind::Sqlite, data_type),
                "SQLite {data_type} must be a built-in type"
            );
            let (_, selected) = create_table_type_select_items(DatabaseKind::Sqlite, data_type);
            assert!(
                selected.is_some(),
                "SQLite {data_type} must select the built-in type"
            );
        }
        assert!(create_table_supports_length(DatabaseKind::Sqlite, "varchar"));
        assert!(
            create_table_provider(DatabaseKind::Sqlite)
                .type_capabilities("decimal")
                .scale
        );
    }
}
