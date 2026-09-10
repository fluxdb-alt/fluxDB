#[derive(Clone, Debug, PartialEq)]
pub struct CreateTableState {
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    pub database_kind: DatabaseKind,
    pub mode: CreateTableMode,
    pub table_name: String,
    pub comment: String,
    pub engine: String,
    pub tablespace: String,
    pub charset: String,
    pub collation: String,
    pub row_format: String,
    pub avg_row_length: String,
    pub max_rows: String,
    pub min_rows: String,
    pub key_block_size: String,
    pub partition_enabled: bool,
    pub partition_method: String,
    pub partition_expression: String,
    pub partition_sql: String,
    pub applying: bool,
    pub apply_error: Option<UserFacingError>,
    pub active_tab: CreateTableTab,
    pub columns: Vec<CreateTableColumn>,
    pub indexes: Vec<CreateTableIndex>,
    pub foreign_keys: Vec<CreateTableForeignKey>,
    pub checks: Vec<CreateTableCheck>,
    pub triggers: Vec<CreateTableTrigger>,
    pub selected_column_id: Option<u64>,
    pub selected_index_id: Option<u64>,
    pub selected_foreign_key_id: Option<u64>,
    pub selected_check_id: Option<u64>,
    pub selected_trigger_id: Option<u64>,
    pub next_column_id: u64,
    pub next_index_id: u64,
    pub next_foreign_key_id: u64,
    pub next_check_id: u64,
    pub next_trigger_id: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CreateTableMode {
    Create,
    Design {
        object: ObjectPath,
        original: CreateTableDesignSnapshot,
        original_ddl: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateTableDesignSnapshot {
    pub comment: String,
    pub engine: String,
    pub tablespace: String,
    pub charset: String,
    pub collation: String,
    pub row_format: String,
    pub avg_row_length: String,
    pub max_rows: String,
    pub min_rows: String,
    pub key_block_size: String,
    pub partition_enabled: bool,
    pub partition_method: String,
    pub partition_expression: String,
    pub partition_sql: String,
    pub columns: Vec<CreateTableColumn>,
    pub indexes: Vec<CreateTableIndex>,
    pub foreign_keys: Vec<CreateTableForeignKey>,
    pub checks: Vec<CreateTableCheck>,
    pub triggers: Vec<CreateTableTrigger>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateTableColumn {
    pub id: u64,
    pub name: String,
    pub data_type: String,
    pub length: String,
    pub scale: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub default_value: String,
    pub comment: String,
    pub auto_increment: bool,
    pub auto_update_time: bool,
    pub unsigned: bool,
    pub zerofill: bool,
    pub binary: bool,
    pub charset: String,
    pub collation: String,
    pub key_length: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateTableIndex {
    pub id: u64,
    pub name: String,
    pub columns: Vec<CreateTableIndexColumn>,
    pub index_type: String,
    pub index_method: String,
    pub comment: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateTableIndexColumn {
    pub name: String,
    pub sub_part: String,
    pub sort_order: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateTableForeignKey {
    pub id: u64,
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_database: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub referenced_column_options: LoadState<Vec<String>>,
    pub on_delete: String,
    pub on_update: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateTableCheck {
    pub id: u64,
    pub name: String,
    pub expression: String,
    pub not_enforced: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateTableTrigger {
    pub id: u64,
    pub name: String,
    pub timing: String,
    pub event: String,
    pub body: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CreateTableTab {
    #[default]
    Fields,
    Indexes,
    ForeignKeys,
    Checks,
    Triggers,
    Options,
    Partitions,
    SqlPreview,
    Ddl,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateTableField {
    TableName,
    Comment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateTableOptionField {
    Engine,
    Tablespace,
    Charset,
    Collation,
    RowFormat,
    AvgRowLength,
    MaxRows,
    MinRows,
    KeyBlockSize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateTablePartitionField {
    Method,
    Expression,
    Sql,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateTableColumnField {
    Name,
    DataType,
    Length,
    Scale,
    DefaultValue,
    Comment,
    Charset,
    Collation,
    KeyLength,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateTableIndexField {
    Name,
    IndexType,
    IndexMethod,
    Comment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateTableIndexColumnField {
    Name,
    SubPart,
    SortOrder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateTableForeignKeyField {
    Name,
    ReferencedDatabase,
    ReferencedTable,
    OnDelete,
    OnUpdate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateTableCheckField {
    Name,
    Expression,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateTableTriggerField {
    Name,
    Timing,
    Body,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateTableTriggerEvent {
    Insert,
    Update,
    Delete,
}

impl CreateTableState {
    pub fn new(
        connection_id: ConnectionId,
        database: Option<String>,
        database_kind: DatabaseKind,
    ) -> Self {
        let provider = create_table_provider(database_kind);
        let (engine, charset) = match database_kind {
            DatabaseKind::MySql | DatabaseKind::TiDb => ("InnoDB", "utf8mb4"),
            _ => ("", ""),
        };
        Self {
            connection_id,
            database,
            database_kind,
            mode: CreateTableMode::Create,
            table_name: String::new(),
            comment: String::new(),
            engine: engine.to_string(),
            tablespace: String::new(),
            charset: charset.to_string(),
            collation: String::new(),
            row_format: String::new(),
            avg_row_length: "0".to_string(),
            max_rows: "0".to_string(),
            min_rows: "0".to_string(),
            key_block_size: "0".to_string(),
            partition_enabled: false,
            partition_method: "RANGE".to_string(),
            partition_expression: String::new(),
            partition_sql: String::new(),
            applying: false,
            apply_error: None,
            active_tab: CreateTableTab::Fields,
            columns: vec![CreateTableColumn::default_id(1, provider)],
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
            checks: Vec::new(),
            triggers: Vec::new(),
            selected_column_id: Some(1),
            selected_index_id: None,
            selected_foreign_key_id: None,
            selected_check_id: None,
            selected_trigger_id: None,
            next_column_id: 2,
            next_index_id: 1,
            next_foreign_key_id: 1,
            next_check_id: 1,
            next_trigger_id: 1,
        }
    }

    pub fn design(
        object: ObjectPath,
        database_kind: DatabaseKind,
        columns: Vec<CompletionColumn>,
        indexes: Vec<IndexInfo>,
        foreign_keys: Vec<ForeignKeyInfo>,
        triggers: Vec<TriggerInfo>,
        ddl: Option<String>,
    ) -> Self {
        let provider = create_table_provider(database_kind);
        let mut create = Self::new(object.connection_id, object.database.clone(), database_kind);
        create.table_name = object.name.clone();
        create.columns = columns
            .into_iter()
            .enumerate()
            .map(|(index, column)| CreateTableColumn::from_completion(index as u64 + 1, column, provider))
            .collect();
        create.indexes = create_table_indexes_from_info(indexes);
        create.foreign_keys = create_table_foreign_keys_from_info(&object, foreign_keys);
        create.triggers = create_table_triggers_from_info(triggers);
        let original_ddl = ddl.clone();
        if let Some(ddl) = ddl {
            create_table_apply_design_ddl(&mut create, &ddl);
        }
        create.selected_column_id = create.columns.first().map(|column| column.id);
        create.selected_index_id = create.indexes.first().map(|index| index.id);
        create.selected_foreign_key_id = create.foreign_keys.first().map(|foreign_key| foreign_key.id);
        create.selected_check_id = create.checks.first().map(|check| check.id);
        create.selected_trigger_id = create.triggers.first().map(|trigger| trigger.id);
        create.next_column_id = create.columns.len() as u64 + 1;
        create.next_index_id = create.indexes.len() as u64 + 1;
        create.next_foreign_key_id = create.foreign_keys.len() as u64 + 1;
        create.next_check_id = create.checks.len() as u64 + 1;
        create.next_trigger_id = create.triggers.len() as u64 + 1;
        let original = create.design_snapshot();
        create.mode = CreateTableMode::Design {
            object,
            original,
            original_ddl,
        };
        create
    }

    pub fn is_design(&self) -> bool {
        matches!(self.mode, CreateTableMode::Design { .. })
    }

    pub fn tab_title(&self) -> String {
        let name = self.table_name.trim();
        if name.is_empty() {
            if self.is_design() {
                "设计表".to_string()
            } else {
                "新建表".to_string()
            }
        } else if self.is_design() {
            format!("设计表: {name}")
        } else {
            name.to_string()
        }
    }

    pub fn add_column(&mut self) {
        let id = self.next_column_id;
        self.next_column_id += 1;
        self.columns
            .push(CreateTableColumn::new(id, create_table_provider(self.database_kind)));
        self.selected_column_id = Some(id);
    }

    pub fn select_column(&mut self, column_id: u64) {
        if self.columns.iter().any(|column| column.id == column_id) {
            self.selected_column_id = Some(column_id);
        }
    }

    pub fn move_column_up(&mut self, column_id: u64) {
        let Some(index) = self.columns.iter().position(|column| column.id == column_id) else {
            return;
        };
        if index > 0 {
            self.columns.swap(index - 1, index);
            self.selected_column_id = Some(column_id);
        }
    }

    pub fn move_column_down(&mut self, column_id: u64) {
        let Some(index) = self.columns.iter().position(|column| column.id == column_id) else {
            return;
        };
        if index + 1 < self.columns.len() {
            self.columns.swap(index, index + 1);
            self.selected_column_id = Some(column_id);
        }
    }

    pub fn remove_column(&mut self, column_id: u64) {
        let Some(index) = self.columns.iter().position(|column| column.id == column_id) else {
            return;
        };
        let old_name = self.columns[index].name.trim().to_string();
        self.columns.remove(index);
        if !old_name.is_empty() {
            for index in &mut self.indexes {
                index
                    .columns
                    .retain(|column| !column.name.eq_ignore_ascii_case(&old_name));
            }
            for foreign_key in &mut self.foreign_keys {
                foreign_key
                    .columns
                    .retain(|column| !column.eq_ignore_ascii_case(&old_name));
            }
        }
        if self.selected_column_id == Some(column_id) {
            self.selected_column_id = self
                .columns
                .get(index)
                .or_else(|| index.checked_sub(1).and_then(|prev| self.columns.get(prev)))
                .map(|column| column.id);
        }
    }

    pub fn selected_column(&self) -> Option<&CreateTableColumn> {
        let selected = self.selected_column_id.or_else(|| self.columns.first().map(|column| column.id))?;
        self.columns.iter().find(|column| column.id == selected)
    }

    pub fn selected_index(&self) -> Option<&CreateTableIndex> {
        let selected = self
            .selected_index_id
            .or_else(|| self.indexes.first().map(|index| index.id))?;
        self.indexes.iter().find(|index| index.id == selected)
    }

    pub fn selected_foreign_key(&self) -> Option<&CreateTableForeignKey> {
        let selected = self
            .selected_foreign_key_id
            .or_else(|| self.foreign_keys.first().map(|foreign_key| foreign_key.id))?;
        self.foreign_keys
            .iter()
            .find(|foreign_key| foreign_key.id == selected)
    }

    pub fn selected_check(&self) -> Option<&CreateTableCheck> {
        let selected = self
            .selected_check_id
            .or_else(|| self.checks.first().map(|check| check.id))?;
        self.checks.iter().find(|check| check.id == selected)
    }

    pub fn selected_trigger(&self) -> Option<&CreateTableTrigger> {
        let selected = self
            .selected_trigger_id
            .or_else(|| self.triggers.first().map(|trigger| trigger.id))?;
        self.triggers.iter().find(|trigger| trigger.id == selected)
    }

    pub fn set_field(&mut self, field: CreateTableField, value: String) {
        match field {
            CreateTableField::TableName => {
                self.table_name = value;
                self.autofill_empty_index_names();
            }
            CreateTableField::Comment => self.comment = value,
        }
    }

    pub fn set_option_field(&mut self, field: CreateTableOptionField, value: String) {
        match field {
            CreateTableOptionField::Engine => {
                self.engine = create_table_normalized_table_engine(&value).to_string()
            }
            CreateTableOptionField::Tablespace => self.tablespace = value,
            CreateTableOptionField::Charset => self.charset = value,
            CreateTableOptionField::Collation => self.collation = value,
            CreateTableOptionField::RowFormat => {
                self.row_format = create_table_normalized_row_format(&value).to_string()
            }
            CreateTableOptionField::AvgRowLength => {
                self.avg_row_length = create_table_digits_or_zero(&value)
            }
            CreateTableOptionField::MaxRows => self.max_rows = create_table_digits_or_zero(&value),
            CreateTableOptionField::MinRows => self.min_rows = create_table_digits_or_zero(&value),
            CreateTableOptionField::KeyBlockSize => {
                self.key_block_size = create_table_digits_or_zero(&value)
            }
        }
    }

    pub fn toggle_partition_enabled(&mut self) {
        self.partition_enabled = !self.partition_enabled;
        if self.partition_enabled && self.partition_sql.trim().is_empty() {
            self.refresh_partition_template();
        }
    }

    pub fn set_partition_field(&mut self, field: CreateTablePartitionField, value: String) {
        match field {
            CreateTablePartitionField::Method => {
                self.partition_method = create_table_normalized_partition_method(&value).to_string();
                self.refresh_partition_template();
            }
            CreateTablePartitionField::Expression => {
                self.partition_expression = value;
                self.refresh_partition_template();
            }
            CreateTablePartitionField::Sql => self.partition_sql = value,
        }
    }

    fn refresh_partition_template(&mut self) {
        let expression = self.partition_expression.trim();
        if expression.is_empty() {
            return;
        }
        self.partition_sql = create_table_partition_template(&self.partition_method, expression);
    }

    pub fn set_column_field(
        &mut self,
        column_id: u64,
        field: CreateTableColumnField,
        value: String,
    ) {
        let provider = create_table_provider(self.database_kind);
        let Some(column) = self.columns.iter_mut().find(|column| column.id == column_id) else {
            return;
        };
        match field {
            CreateTableColumnField::Name => {
                let old_name = column.name.trim().to_string();
                column.name = value.clone();
                let new_name = value.trim();
                if !old_name.is_empty() && !new_name.is_empty() {
                    for index in &mut self.indexes {
                        for column in &mut index.columns {
                            if column.name.eq_ignore_ascii_case(&old_name) {
                                column.name = new_name.to_string();
                            }
                        }
                    }
                    for foreign_key in &mut self.foreign_keys {
                        for column in &mut foreign_key.columns {
                            if column.eq_ignore_ascii_case(&old_name) {
                                *column = new_name.to_string();
                            }
                        }
                    }
                    self.autofill_empty_index_names();
                    self.autofill_empty_foreign_key_names();
                }
            }
            CreateTableColumnField::DataType => {
                column.set_data_type(provider, value);
            }
            CreateTableColumnField::Length => {
                if provider.type_capabilities(&column.data_type).length {
                    column.length = create_table_digits_only(&value);
                } else {
                    column.length.clear();
                }
            }
            CreateTableColumnField::Scale => {
                if provider.type_capabilities(&column.data_type).scale {
                    column.scale = create_table_digits_only(&value);
                } else {
                    column.scale.clear();
                }
            }
            CreateTableColumnField::DefaultValue => column.default_value = value,
            CreateTableColumnField::Comment => column.comment = value,
            CreateTableColumnField::Charset => {
                if provider.type_capabilities(&column.data_type).text_options {
                    column.charset = value;
                } else {
                    column.charset.clear();
                }
            }
            CreateTableColumnField::Collation => {
                if provider.type_capabilities(&column.data_type).text_options {
                    column.collation = value;
                } else {
                    column.collation.clear();
                }
            }
            CreateTableColumnField::KeyLength => {
                if column.supports_primary_key_prefix_length() {
                    column.key_length = create_table_digits_only(&value);
                } else {
                    column.key_length.clear();
                }
            }
        }
    }

    pub fn add_index(&mut self) {
        let id = self.next_index_id;
        self.next_index_id += 1;
        let mut index = CreateTableIndex::new(id);
        index
            .columns
            .push(CreateTableIndexColumn::new(String::new()));
        self.indexes.push(index);
        self.selected_index_id = Some(id);
        self.autofill_empty_index_names();
    }

    pub fn add_check(&mut self) {
        let id = self.next_check_id;
        self.next_check_id += 1;
        self.checks.push(CreateTableCheck::new(id));
        self.selected_check_id = Some(id);
    }

    pub fn select_check(&mut self, check_id: u64) {
        if self.checks.iter().any(|check| check.id == check_id) {
            self.selected_check_id = Some(check_id);
        }
    }

    pub fn move_check_up(&mut self, check_id: u64) {
        let Some(index) = self.checks.iter().position(|item| item.id == check_id) else {
            return;
        };
        if index > 0 {
            self.checks.swap(index - 1, index);
            self.selected_check_id = Some(check_id);
        }
    }

    pub fn move_check_down(&mut self, check_id: u64) {
        let Some(index) = self.checks.iter().position(|item| item.id == check_id) else {
            return;
        };
        if index + 1 < self.checks.len() {
            self.checks.swap(index, index + 1);
            self.selected_check_id = Some(check_id);
        }
    }

    pub fn remove_check(&mut self, check_id: u64) {
        let Some(index) = self.checks.iter().position(|item| item.id == check_id) else {
            return;
        };
        self.checks.remove(index);
        if self.selected_check_id == Some(check_id) {
            self.selected_check_id = self
                .checks
                .get(index)
                .or_else(|| index.checked_sub(1).and_then(|prev| self.checks.get(prev)))
                .map(|check| check.id);
        }
    }

    pub fn set_check_field(
        &mut self,
        check_id: u64,
        field: CreateTableCheckField,
        value: String,
    ) {
        let Some(check) = self.checks.iter_mut().find(|check| check.id == check_id) else {
            return;
        };
        match field {
            CreateTableCheckField::Name => check.name = value,
            CreateTableCheckField::Expression => check.expression = value,
        }
    }

    pub fn toggle_check_not_enforced(&mut self, check_id: u64) {
        if self.database_kind != DatabaseKind::MySql && self.database_kind != DatabaseKind::TiDb {
            return;
        }
        if let Some(check) = self.checks.iter_mut().find(|check| check.id == check_id) {
            check.not_enforced = !check.not_enforced;
        }
    }

    pub fn add_foreign_key(&mut self) {
        create_table_add_foreign_key(self);
    }

    pub fn add_trigger(&mut self) {
        let id = self.next_trigger_id;
        self.next_trigger_id += 1;
        self.triggers.push(CreateTableTrigger::new(id));
        self.selected_trigger_id = Some(id);
    }

    pub fn select_trigger(&mut self, trigger_id: u64) {
        if self.triggers.iter().any(|trigger| trigger.id == trigger_id) {
            self.selected_trigger_id = Some(trigger_id);
        }
    }

    pub fn move_trigger_up(&mut self, trigger_id: u64) {
        let Some(index) = self.triggers.iter().position(|item| item.id == trigger_id) else {
            return;
        };
        if index > 0 {
            self.triggers.swap(index - 1, index);
            self.selected_trigger_id = Some(trigger_id);
        }
    }

    pub fn move_trigger_down(&mut self, trigger_id: u64) {
        let Some(index) = self.triggers.iter().position(|item| item.id == trigger_id) else {
            return;
        };
        if index + 1 < self.triggers.len() {
            self.triggers.swap(index, index + 1);
            self.selected_trigger_id = Some(trigger_id);
        }
    }

    pub fn remove_trigger(&mut self, trigger_id: u64) {
        let Some(index) = self.triggers.iter().position(|item| item.id == trigger_id) else {
            return;
        };
        self.triggers.remove(index);
        if self.selected_trigger_id == Some(trigger_id) {
            self.selected_trigger_id = self
                .triggers
                .get(index)
                .or_else(|| index.checked_sub(1).and_then(|prev| self.triggers.get(prev)))
                .map(|trigger| trigger.id);
        }
    }

    pub fn set_trigger_field(
        &mut self,
        trigger_id: u64,
        field: CreateTableTriggerField,
        value: String,
    ) {
        let Some(trigger) = self.triggers.iter_mut().find(|trigger| trigger.id == trigger_id)
        else {
            return;
        };
        match field {
            CreateTableTriggerField::Name => trigger.name = value,
            CreateTableTriggerField::Timing => {
                trigger.timing = create_table_normalized_trigger_timing(&value).to_string()
            }
            CreateTableTriggerField::Body => trigger.body = value,
        }
    }

    pub fn set_trigger_event(&mut self, trigger_id: u64, event: CreateTableTriggerEvent) {
        let Some(trigger) = self.triggers.iter_mut().find(|trigger| trigger.id == trigger_id)
        else {
            return;
        };
        trigger.event = event.as_sql().to_string();
    }

    pub fn select_foreign_key(&mut self, foreign_key_id: u64) {
        if self
            .foreign_keys
            .iter()
            .any(|foreign_key| foreign_key.id == foreign_key_id)
        {
            self.selected_foreign_key_id = Some(foreign_key_id);
        }
    }

    pub fn move_foreign_key_up(&mut self, foreign_key_id: u64) {
        let Some(index) = self
            .foreign_keys
            .iter()
            .position(|item| item.id == foreign_key_id)
        else {
            return;
        };
        if index > 0 {
            self.foreign_keys.swap(index - 1, index);
            self.selected_foreign_key_id = Some(foreign_key_id);
        }
    }

    pub fn move_foreign_key_down(&mut self, foreign_key_id: u64) {
        let Some(index) = self
            .foreign_keys
            .iter()
            .position(|item| item.id == foreign_key_id)
        else {
            return;
        };
        if index + 1 < self.foreign_keys.len() {
            self.foreign_keys.swap(index, index + 1);
            self.selected_foreign_key_id = Some(foreign_key_id);
        }
    }

    pub fn remove_foreign_key(&mut self, foreign_key_id: u64) {
        let Some(index) = self
            .foreign_keys
            .iter()
            .position(|item| item.id == foreign_key_id)
        else {
            return;
        };
        self.foreign_keys.remove(index);
        if self.selected_foreign_key_id == Some(foreign_key_id) {
            self.selected_foreign_key_id = self
                .foreign_keys
                .get(index)
                .or_else(|| {
                    index
                        .checked_sub(1)
                        .and_then(|prev| self.foreign_keys.get(prev))
                })
                .map(|foreign_key| foreign_key.id);
        }
    }

    pub fn set_foreign_key_field(
        &mut self,
        foreign_key_id: u64,
        field: CreateTableForeignKeyField,
        value: String,
    ) {
        create_table_set_foreign_key_field(self, foreign_key_id, field, value);
    }

    pub fn add_foreign_key_column(&mut self, foreign_key_id: u64) {
        create_table_add_foreign_key_column(self, foreign_key_id);
    }

    pub fn move_foreign_key_column_up(&mut self, foreign_key_id: u64, column_index: usize) {
        create_table_move_foreign_key_column_up(self, foreign_key_id, column_index);
    }

    pub fn move_foreign_key_column_down(&mut self, foreign_key_id: u64, column_index: usize) {
        create_table_move_foreign_key_column_down(self, foreign_key_id, column_index);
    }

    pub fn remove_foreign_key_column(&mut self, foreign_key_id: u64, column_index: usize) {
        create_table_remove_foreign_key_column(self, foreign_key_id, column_index);
    }

    pub fn set_foreign_key_column(
        &mut self,
        foreign_key_id: u64,
        column_index: usize,
        value: String,
    ) {
        create_table_set_foreign_key_column(self, foreign_key_id, column_index, value);
    }

    pub fn start_foreign_key_reference_columns_load(&mut self, foreign_key_id: u64) {
        create_table_start_foreign_key_reference_columns_load(self, foreign_key_id);
    }

    pub fn finish_foreign_key_reference_columns_load(
        &mut self,
        foreign_key_id: u64,
        result: std::result::Result<Vec<String>, UserFacingError>,
    ) {
        create_table_finish_foreign_key_reference_columns_load(self, foreign_key_id, result);
    }

    pub fn add_foreign_key_referenced_column(&mut self, foreign_key_id: u64) {
        create_table_add_foreign_key_referenced_column(self, foreign_key_id);
    }

    pub fn move_foreign_key_referenced_column_up(
        &mut self,
        foreign_key_id: u64,
        column_index: usize,
    ) {
        create_table_move_foreign_key_referenced_column_up(self, foreign_key_id, column_index);
    }

    pub fn move_foreign_key_referenced_column_down(
        &mut self,
        foreign_key_id: u64,
        column_index: usize,
    ) {
        create_table_move_foreign_key_referenced_column_down(self, foreign_key_id, column_index);
    }

    pub fn remove_foreign_key_referenced_column(
        &mut self,
        foreign_key_id: u64,
        column_index: usize,
    ) {
        create_table_remove_foreign_key_referenced_column(self, foreign_key_id, column_index);
    }

    pub fn set_foreign_key_referenced_column(
        &mut self,
        foreign_key_id: u64,
        column_index: usize,
        value: String,
    ) {
        create_table_set_foreign_key_referenced_column(
            self,
            foreign_key_id,
            column_index,
            value,
        );
    }

    pub fn select_index(&mut self, index_id: u64) {
        if self.indexes.iter().any(|index| index.id == index_id) {
            self.selected_index_id = Some(index_id);
        }
    }

    pub fn move_index_up(&mut self, index_id: u64) {
        let Some(index) = self.indexes.iter().position(|item| item.id == index_id) else {
            return;
        };
        if index > 0 {
            self.indexes.swap(index - 1, index);
            self.selected_index_id = Some(index_id);
        }
    }

    pub fn move_index_down(&mut self, index_id: u64) {
        let Some(index) = self.indexes.iter().position(|item| item.id == index_id) else {
            return;
        };
        if index + 1 < self.indexes.len() {
            self.indexes.swap(index, index + 1);
            self.selected_index_id = Some(index_id);
        }
    }

    pub fn remove_index(&mut self, index_id: u64) {
        let Some(index) = self.indexes.iter().position(|item| item.id == index_id) else {
            return;
        };
        self.indexes.remove(index);
        if self.selected_index_id == Some(index_id) {
            self.selected_index_id = self
                .indexes
                .get(index)
                .or_else(|| index.checked_sub(1).and_then(|prev| self.indexes.get(prev)))
                .map(|index| index.id);
        }
    }

    pub fn set_index_field(
        &mut self,
        index_id: u64,
        field: CreateTableIndexField,
        value: String,
    ) {
        let Some(index) = self.indexes.iter_mut().find(|index| index.id == index_id) else {
            return;
        };
        match field {
            CreateTableIndexField::Name => index.name = value,
            CreateTableIndexField::IndexType => {
                index.index_type = if value.trim().is_empty() {
                    String::new()
                } else {
                    create_table_normalized_index_type(&value).to_string()
                };
                if !create_table_index_type_supports_method(&index.index_type) {
                    index.index_method.clear();
                }
                if index.name.trim().is_empty() {
                    self.autofill_empty_index_names();
                }
            }
            CreateTableIndexField::IndexMethod => {
                index.index_method = if value.trim().is_empty()
                    || !create_table_index_type_supports_method(&index.index_type)
                {
                    String::new()
                } else {
                    create_table_normalized_index_method(&value).to_string()
                }
            }
            CreateTableIndexField::Comment => index.comment = value,
        }
    }

    pub fn add_index_column(&mut self, index_id: u64) {
        let used = self
            .indexes
            .iter()
            .find(|index| index.id == index_id)
            .map(|index| {
                index
                    .columns
                    .iter()
                    .map(|column| column.name.to_ascii_lowercase())
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let next_column = self
            .columns
            .iter()
            .map(|column| column.name.trim())
            .find(|name| !name.is_empty() && !used.contains(&name.to_ascii_lowercase()))
            .map(str::to_string)
            .unwrap_or_default();
        let Some(index) = self.indexes.iter_mut().find(|index| index.id == index_id) else {
            return;
        };
        index.columns.push(CreateTableIndexColumn::new(next_column));
        self.selected_index_id = Some(index_id);
        self.autofill_empty_index_names();
    }

    pub fn move_index_column_up(&mut self, index_id: u64, column_index: usize) {
        let Some(index) = self.indexes.iter_mut().find(|index| index.id == index_id) else {
            return;
        };
        if column_index > 0 && column_index < index.columns.len() {
            index.columns.swap(column_index - 1, column_index);
        }
    }

    pub fn move_index_column_down(&mut self, index_id: u64, column_index: usize) {
        let Some(index) = self.indexes.iter_mut().find(|index| index.id == index_id) else {
            return;
        };
        if column_index + 1 < index.columns.len() {
            index.columns.swap(column_index, column_index + 1);
        }
    }

    pub fn remove_index_column(&mut self, index_id: u64, column_index: usize) {
        let Some(index) = self.indexes.iter_mut().find(|index| index.id == index_id) else {
            return;
        };
        if column_index < index.columns.len() {
            index.columns.remove(column_index);
        }
    }

    pub fn set_index_column_field(
        &mut self,
        index_id: u64,
        column_index: usize,
        field: CreateTableIndexColumnField,
        value: String,
    ) {
        let Some(index) = self.indexes.iter_mut().find(|index| index.id == index_id) else {
            return;
        };
        let Some(column) = index.columns.get_mut(column_index) else {
            return;
        };
        match field {
            CreateTableIndexColumnField::Name => column.name = value,
            CreateTableIndexColumnField::SubPart => {
                column.sub_part = create_table_digits_only(&value)
            }
            CreateTableIndexColumnField::SortOrder => {
                column.sort_order = create_table_normalized_sort_order(&value).to_string()
            }
        }
        self.autofill_empty_index_names();
    }

    pub fn toggle_column_flag(&mut self, column_id: u64, flag: CreateTableColumnFlag) {
        let provider = create_table_provider(self.database_kind);
        let Some(column) = self.columns.iter_mut().find(|column| column.id == column_id) else {
            return;
        };
        match flag {
            CreateTableColumnFlag::Nullable => column.nullable = !column.nullable,
            CreateTableColumnFlag::PrimaryKey => {
                column.primary_key = !column.primary_key;
                if !provider.type_capabilities(&column.data_type).key_length {
                    column.key_length.clear();
                }
            }
            CreateTableColumnFlag::AutoIncrement => {
                if provider.type_capabilities(&column.data_type).auto_increment {
                    column.auto_increment = !column.auto_increment;
                }
            }
            CreateTableColumnFlag::AutoUpdateTime => {
                if provider.type_capabilities(&column.data_type).auto_update_time {
                    column.auto_update_time = !column.auto_update_time;
                }
            }
            CreateTableColumnFlag::Unsigned => {
                if provider.type_capabilities(&column.data_type).unsigned {
                    column.unsigned = !column.unsigned;
                }
            }
            CreateTableColumnFlag::Zerofill => {
                if provider.type_capabilities(&column.data_type).zerofill {
                    column.zerofill = !column.zerofill;
                }
            }
            CreateTableColumnFlag::Binary => {
                if provider.type_capabilities(&column.data_type).binary_attribute {
                    column.binary = !column.binary;
                }
            }
        }
    }

    pub fn sql_preview(&self) -> Result<String, String> {
        match self.mode {
            CreateTableMode::Create => create_table_provider(self.database_kind).sql_preview(self),
            CreateTableMode::Design { .. } => {
                create_table_provider(self.database_kind).design_sql_preview(self)
            }
        }
    }

    pub fn ddl_preview(&self) -> Result<String, String> {
        match &self.mode {
            CreateTableMode::Design {
                original_ddl: Some(ddl),
                ..
            } if !ddl.trim().is_empty() => Ok(ddl.clone()),
            CreateTableMode::Design { .. } => Err("表 DDL 不存在".to_string()),
            CreateTableMode::Create => create_table_provider(self.database_kind).sql_preview(self),
        }
    }

    pub fn validation_error(&self) -> Option<&'static str> {
        if let Some(message) = create_table_provider(self.database_kind).validation_error(self) {
            return Some(message);
        }
        if self.is_design()
            && create_table_provider(self.database_kind)
                .design_statements(self)
                .is_ok_and(|sql| sql.is_empty())
        {
            return Some("没有需要保存的变更");
        }
        None
    }

    pub fn type_options(&self) -> Vec<String> {
        create_table_provider(self.database_kind)
            .type_options()
            .iter()
            .map(|value| value.to_string())
            .collect()
    }

    pub fn type_capabilities(&self, data_type: &str) -> CreateTableTypeCapabilities {
        create_table_provider(self.database_kind).type_capabilities(data_type)
    }

    pub fn length_placeholder(&self, data_type: &str) -> &'static str {
        create_table_provider(self.database_kind)
            .default_length(data_type)
            .unwrap_or("")
    }

    fn autofill_empty_index_names(&mut self) {
        let table_name = self.table_name.trim();
        if table_name.is_empty() {
            return;
        }
        for index in &mut self.indexes {
            if index.name.trim().is_empty() {
                index.name = create_table_auto_index_name(
                    table_name,
                    &index.index_type,
                    index
                        .columns
                        .iter()
                        .map(|column| column.name.trim())
                        .filter(|name| !name.is_empty()),
                );
            }
        }
    }

    fn autofill_empty_foreign_key_names(&mut self) {
        create_table_autofill_empty_foreign_key_names(self);
    }

    fn design_snapshot(&self) -> CreateTableDesignSnapshot {
        CreateTableDesignSnapshot {
            comment: self.comment.clone(),
            engine: self.engine.clone(),
            tablespace: self.tablespace.clone(),
            charset: self.charset.clone(),
            collation: self.collation.clone(),
            row_format: self.row_format.clone(),
            avg_row_length: self.avg_row_length.clone(),
            max_rows: self.max_rows.clone(),
            min_rows: self.min_rows.clone(),
            key_block_size: self.key_block_size.clone(),
            partition_enabled: self.partition_enabled,
            partition_method: self.partition_method.clone(),
            partition_expression: self.partition_expression.clone(),
            partition_sql: self.partition_sql.clone(),
            columns: self.columns.clone(),
            indexes: self.indexes.clone(),
            foreign_keys: self.foreign_keys.clone(),
            checks: self.checks.clone(),
            triggers: self.triggers.clone(),
        }
    }
}

fn create_table_indexes_from_info(indexes: Vec<IndexInfo>) -> Vec<CreateTableIndex> {
    indexes
        .into_iter()
        .filter(|index| !index.is_primary && !index.name.eq_ignore_ascii_case("PRIMARY"))
        .enumerate()
        .map(|(position, info)| CreateTableIndex {
            id: position as u64 + 1,
            name: info.name,
            columns: info
                .columns
                .into_iter()
                .map(CreateTableIndexColumn::new)
                .collect(),
            index_type: if info.is_unique { "UNIQUE" } else { "NORMAL" }.to_string(),
            index_method: info
                .index_type
                .filter(|value| matches!(value.to_ascii_uppercase().as_str(), "BTREE" | "HASH"))
                .unwrap_or_default(),
            comment: info.comment.unwrap_or_default(),
        })
        .collect()
}

fn create_table_foreign_keys_from_info(
    object: &ObjectPath,
    foreign_keys: Vec<ForeignKeyInfo>,
) -> Vec<CreateTableForeignKey> {
    let mut grouped: BTreeMap<String, CreateTableForeignKey> = BTreeMap::new();
    for info in foreign_keys {
        let next_id = grouped.len() as u64 + 1;
        let key = info.name.clone();
        let entry = grouped.entry(key).or_insert_with(|| CreateTableForeignKey {
            id: next_id,
            name: info.name,
            columns: Vec::new(),
            referenced_database: info
                .ref_schema
                .clone()
                .filter(|schema| object.database.as_deref() != Some(schema))
                .unwrap_or_default(),
            referenced_table: info.ref_table,
            referenced_columns: Vec::new(),
            referenced_column_options: LoadState::NotLoaded,
            on_delete: String::new(),
            on_update: String::new(),
        });
        entry.columns.push(info.column);
        entry.referenced_columns.push(info.ref_column);
    }
    grouped.into_values().collect()
}

fn create_table_triggers_from_info(triggers: Vec<TriggerInfo>) -> Vec<CreateTableTrigger> {
    triggers
        .into_iter()
        .enumerate()
        .map(|(position, trigger)| CreateTableTrigger {
            id: position as u64 + 1,
            name: trigger.name,
            timing: create_table_normalized_trigger_timing(&trigger.timing).to_string(),
            event: create_table_normalized_trigger_event(&trigger.event).to_string(),
            body: trigger.body.unwrap_or_default(),
        })
        .collect()
}

fn create_table_apply_design_ddl(create: &mut CreateTableState, ddl: &str) {
    if matches!(create.database_kind, DatabaseKind::MySql | DatabaseKind::TiDb) {
        create_table_apply_mysql_table_options(create, ddl);
        create_table_apply_mysql_partition(create, ddl);
    }
    create.checks = create_table_checks_from_ddl(ddl);
}

fn create_table_apply_mysql_table_options(create: &mut CreateTableState, ddl: &str) {
    let options = create_table_ddl_suffix(ddl);
    if let Some(value) = create_table_option_value(options, "ENGINE") {
        create.engine = value;
    }
    if let Some(value) = create_table_option_value(options, "TABLESPACE") {
        create.tablespace = create_table_unquote_identifier(&value);
    }
    if let Some(value) = create_table_option_value(options, "CHARSET")
        .or_else(|| create_table_option_value(options, "DEFAULT CHARSET"))
    {
        create.charset = value;
    }
    if let Some(value) = create_table_option_value(options, "COLLATE") {
        create.collation = value;
    }
    if let Some(value) = create_table_option_value(options, "ROW_FORMAT") {
        create.row_format = value;
    }
    if let Some(value) = create_table_option_value(options, "COMMENT") {
        create.comment = value;
    }
    for (name, field) in [
        ("AVG_ROW_LENGTH", &mut create.avg_row_length),
        ("MAX_ROWS", &mut create.max_rows),
        ("MIN_ROWS", &mut create.min_rows),
        ("KEY_BLOCK_SIZE", &mut create.key_block_size),
    ] {
        if let Some(value) = create_table_option_value(options, name) {
            *field = create_table_digits_or_zero(&value);
        }
    }
}

fn create_table_apply_mysql_partition(create: &mut CreateTableState, ddl: &str) {
    let Some(start) = create_table_find_ci(ddl, "PARTITION BY ") else {
        return;
    };
    let sql = ddl[start..].trim().trim_end_matches(';').to_string();
    create.partition_enabled = true;
    create.partition_sql = sql;
    let method = create.partition_sql["PARTITION BY ".len()..]
        .split_whitespace()
        .next()
        .unwrap_or("");
    create.partition_method = create_table_normalized_partition_method(method).to_string();
}

fn create_table_checks_from_ddl(ddl: &str) -> Vec<CreateTableCheck> {
    create_table_ddl_entries(ddl)
        .into_iter()
        .filter_map(|entry| create_table_check_from_ddl_entry(&entry))
        .enumerate()
        .map(|(position, mut check)| {
            check.id = position as u64 + 1;
            check
        })
        .collect()
}

fn create_table_check_from_ddl_entry(entry: &str) -> Option<CreateTableCheck> {
    let check_pos = create_table_find_ci(entry, "CHECK")?;
    let before = entry[..check_pos].trim();
    let name = create_table_constraint_name(before).unwrap_or_default();
    let after = entry[check_pos + "CHECK".len()..].trim();
    let mut expression = create_table_parenthesized(after)?;
    let not_enforced = create_table_find_ci(&expression, "NOT ENFORCED").is_some()
        || create_table_find_ci(after, "NOT ENFORCED").is_some();
    if let Some(pos) = create_table_find_ci(&expression, "NOT ENFORCED") {
        expression = expression[..pos].trim().to_string();
    }
    Some(CreateTableCheck {
        id: 0,
        name,
        expression,
        not_enforced,
    })
}

fn create_table_constraint_name(before: &str) -> Option<String> {
    let pos = create_table_find_ci(before, "CONSTRAINT")?;
    let name = before[pos + "CONSTRAINT".len()..].trim();
    Some(create_table_unquote_identifier(name.split_whitespace().next().unwrap_or("")))
        .filter(|name| !name.is_empty())
}

fn create_table_ddl_entries(ddl: &str) -> Vec<String> {
    let Some(start) = ddl.find('(') else {
        return Vec::new();
    };
    let Some(end) = create_table_matching_paren(ddl, start) else {
        return Vec::new();
    };
    create_table_split_top_level(&ddl[start + 1..end])
}

fn create_table_ddl_suffix(ddl: &str) -> &str {
    let Some(start) = ddl.find('(') else {
        return "";
    };
    let Some(end) = create_table_matching_paren(ddl, start) else {
        return "";
    };
    &ddl[end + 1..]
}

fn create_table_option_value(options: &str, name: &str) -> Option<String> {
    let pos = create_table_find_ci(options, name)?;
    let mut rest = options[pos + name.len()..].trim_start();
    if let Some(stripped) = rest.strip_prefix('=') {
        rest = stripped.trim_start();
    }
    let value = if let Some(quote) = rest.chars().next().filter(|ch| matches!(ch, '\'' | '"' | '`')) {
        let end = rest[1..].find(quote).map(|index| index + 2).unwrap_or(rest.len());
        &rest[..end]
    } else {
        rest.split_whitespace().next().unwrap_or("").trim_matches(',')
    };
    Some(create_table_unquote_identifier(value)).filter(|value| !value.is_empty())
}

fn create_table_parenthesized(text: &str) -> Option<String> {
    let start = text.find('(')?;
    let end = create_table_matching_paren(text, start)?;
    Some(text[start + 1..end].trim().to_string())
}

fn create_table_matching_paren(text: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    for (index, ch) in text.char_indices().skip_while(|(index, _)| *index < start) {
        if let Some(end_quote) = quote {
            if ch == end_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' | '`' => quote = Some(ch),
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn create_table_split_top_level(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut quote = None;
    for (index, ch) in text.char_indices() {
        if let Some(end_quote) = quote {
            if ch == end_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' | '`' => quote = Some(ch),
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let part = text[start..index].trim();
                if !part.is_empty() {
                    parts.push(part.to_string());
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    let part = text[start..].trim();
    if !part.is_empty() {
        parts.push(part.to_string());
    }
    parts
}

fn create_table_find_ci(haystack: &str, needle: &str) -> Option<usize> {
    haystack.to_ascii_uppercase().find(&needle.to_ascii_uppercase())
}

fn create_table_unquote_identifier(value: &str) -> String {
    value
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateTableColumnFlag {
    Nullable,
    PrimaryKey,
    AutoIncrement,
    AutoUpdateTime,
    Unsigned,
    Zerofill,
    Binary,
}

impl CreateTableColumn {
    fn default_id(id: u64, provider: &dyn CreateTableProvider) -> Self {
        let mut column = Self::new(id, provider);
        column.name = "id".to_string();
        column.data_type = provider.default_id_type().to_string();
        column.length = provider.default_length(&column.data_type).unwrap_or("").to_string();
        column.nullable = false;
        column.primary_key = true;
        column
    }

    fn new(id: u64, provider: &dyn CreateTableProvider) -> Self {
        let data_type = provider.default_column_type().to_string();
        Self {
            id,
            name: String::new(),
            length: provider.default_length(&data_type).unwrap_or("").to_string(),
            data_type,
            scale: String::new(),
            nullable: true,
            primary_key: false,
            default_value: String::new(),
            comment: String::new(),
            auto_increment: false,
            auto_update_time: false,
            unsigned: false,
            zerofill: false,
            binary: false,
            charset: String::new(),
            collation: String::new(),
            key_length: String::new(),
        }
    }

    fn from_completion(
        id: u64,
        column: CompletionColumn,
        provider: &dyn CreateTableProvider,
    ) -> Self {
        let raw_type = column
            .type_name
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| provider.default_column_type());
        let (data_type, length, scale) = create_table_split_column_type(raw_type);
        Self {
            id,
            name: column.name,
            data_type,
            length,
            scale,
            nullable: column.nullable,
            primary_key: column.primary_key,
            default_value: String::new(),
            comment: column.comment.unwrap_or_default(),
            auto_increment: false,
            auto_update_time: false,
            unsigned: raw_type.to_ascii_lowercase().contains(" unsigned"),
            zerofill: raw_type.to_ascii_lowercase().contains(" zerofill"),
            binary: false,
            charset: String::new(),
            collation: String::new(),
            key_length: String::new(),
        }
    }

    fn set_data_type(&mut self, provider: &dyn CreateTableProvider, data_type: String) {
        let old_default = provider.default_length(&self.data_type);
        let old_length = self.length.trim().to_string();
        self.data_type = data_type;

        if provider.type_capabilities(&self.data_type).length {
            if let Some(default_length) = provider.default_length(&self.data_type)
                && (old_length.is_empty() || Some(old_length.as_str()) == old_default)
            {
                self.length = default_length.to_string();
            }
        } else {
            self.length.clear();
        }

        if !provider.type_capabilities(&self.data_type).scale {
            self.scale.clear();
        }
        if !provider.type_capabilities(&self.data_type).key_length {
            self.key_length.clear();
        }
        if !provider.type_capabilities(&self.data_type).text_options {
            self.binary = false;
            self.charset.clear();
            self.collation.clear();
        }
        self.normalize_type_flags(provider);
    }

    fn normalize_type_flags(&mut self, provider: &dyn CreateTableProvider) {
        let capabilities = provider.type_capabilities(&self.data_type);
        if !capabilities.auto_increment {
            self.auto_increment = false;
        }
        if !capabilities.auto_update_time {
            self.auto_update_time = false;
        }
        if !capabilities.unsigned {
            self.unsigned = false;
        }
        if !capabilities.zerofill {
            self.zerofill = false;
        }
    }

    fn sql_line(&self) -> Option<String> {
        let name = self.name.trim();
        if name.is_empty() {
            return None;
        }
        let mut sql = format!(
            "{} {}",
            quote_mysql_identifier(name),
            create_table_column_type(&self.data_type, &self.length, &self.scale)
        );
        if self.unsigned {
            sql.push_str(" UNSIGNED");
        }
        if self.zerofill {
            sql.push_str(" ZEROFILL");
        }
        if create_table_is_text_type(&self.data_type) {
            if self.binary {
                sql.push_str(" BINARY");
            }
            if !self.charset.trim().is_empty() {
                sql.push_str(" CHARACTER SET ");
                sql.push_str(self.charset.trim());
            }
            if !self.collation.trim().is_empty() {
                sql.push_str(" COLLATE ");
                sql.push_str(self.collation.trim());
            }
        }
        if !self.nullable || self.primary_key {
            sql.push_str(" NOT NULL");
        }
        if self.auto_increment && create_table_is_number_type(&self.data_type) {
            sql.push_str(" AUTO_INCREMENT");
        }
        if !self.default_value.trim().is_empty() {
            sql.push_str(" DEFAULT ");
            sql.push_str(self.default_value.trim());
        }
        if self.auto_update_time && create_table_is_auto_update_time_type(&self.data_type) {
            sql.push_str(" ON UPDATE CURRENT_TIMESTAMP");
        }
        if !self.comment.trim().is_empty() {
            sql.push_str(" COMMENT ");
            sql.push_str(&quote_mysql_string(self.comment.trim()));
        }
        Some(sql)
    }

    fn supports_primary_key_prefix_length(&self) -> bool {
        self.primary_key && create_table_supports_key_length(&self.data_type)
    }
}

fn create_table_mysql_sql_preview(create: &CreateTableState) -> Result<String, String> {
    if let Some(message) = create.validation_error() {
        return Err(message.to_string());
    }

    let table_name = create.table_name.trim();
    let mut lines = create
        .columns
        .iter()
        .filter_map(CreateTableColumn::sql_line)
        .collect::<Vec<_>>();
    if let Some(primary_key) = create_table_primary_key_line(&create.columns) {
        lines.push(primary_key);
    }
    lines.extend(create_table_index_lines(create)?);
    lines.extend(create_table_foreign_key_lines(
        create,
        CreateTableSqlDialect::MySql,
    )?);
    lines.extend(create_table_check_lines(
        create,
        CreateTableSqlDialect::MySql,
    )?);

    let mut sql = format!(
        "CREATE TABLE {} (\n{}\n)",
        quote_mysql_identifier(table_name),
        lines
            .into_iter()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join(",\n")
    );
    let comment = create.comment.trim();
    if !comment.is_empty() {
        sql.push_str(&format!(" COMMENT={}", quote_mysql_string(comment)));
    }
    let table_options = create_table_mysql_table_options(create);
    if !table_options.is_empty() {
        sql.push(' ');
        sql.push_str(&table_options.join(" "));
    }
    let partition_sql = create_table_mysql_partition_sql(create);
    if !partition_sql.is_empty() {
        sql.push('\n');
        sql.push_str(&partition_sql);
    }
    sql.push(';');
    let trigger_statements = create_table_mysql_trigger_statements(create)?;
    if !trigger_statements.is_empty() {
        sql.push('\n');
        sql.push_str(&trigger_statements.join("\n"));
    }
    Ok(sql)
}

fn create_table_sqlite_sql_preview(create: &CreateTableState) -> Result<String, String> {
    if let Some(message) = create.validation_error() {
        return Err(message.to_string());
    }

    let mut sql = create_table_sqlite_create_table_statement(create, create.table_name.trim())?;
    let index_statements = create_table_sqlite_index_statements(create)?;
    if !index_statements.is_empty() {
        sql.push('\n');
        sql.push_str(&index_statements.join("\n"));
    }
    let trigger_statements = create_table_sqlite_trigger_statements(create)?;
    if !trigger_statements.is_empty() {
        sql.push('\n');
        sql.push_str(&trigger_statements.join("\n"));
    }
    Ok(sql)
}

fn create_table_sqlite_create_table_statement(
    create: &CreateTableState,
    table_name: &str,
) -> Result<String, String> {
    let primary_key_columns = create
        .columns
        .iter()
        .filter(|column| column.primary_key && !column.name.trim().is_empty())
        .collect::<Vec<_>>();
    let inline_auto_increment_id = primary_key_columns.len() == 1
        && primary_key_columns[0].auto_increment
        && matches!(
            create_table_base_type(&primary_key_columns[0].data_type).as_str(),
            "integer" | "int"
        );

    let mut lines = create
        .columns
        .iter()
        .filter_map(|column| {
            create_table_sqlite_column_line(column, inline_auto_increment_id)
        })
        .collect::<Vec<_>>();
    if !inline_auto_increment_id {
        let primary_key = primary_key_columns
            .iter()
            .map(|column| quote_sqlite_identifier(column.name.trim()))
            .collect::<Vec<_>>();
        if !primary_key.is_empty() {
            lines.push(format!("PRIMARY KEY ({})", primary_key.join(", ")));
        }
    }
    lines.extend(create_table_foreign_key_lines(
        create,
        CreateTableSqlDialect::Sqlite,
    )?);
    lines.extend(create_table_check_lines(
        create,
        CreateTableSqlDialect::Sqlite,
    )?);

    Ok(format!(
        "CREATE TABLE {table_name} (\n{}\n);",
        lines
            .into_iter()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join(",\n"),
        table_name = quote_sqlite_identifier(table_name)
    ))
}

fn create_table_sqlite_column_line(
    column: &CreateTableColumn,
    inline_auto_increment_id: bool,
) -> Option<String> {
    let name = column.name.trim();
    if name.is_empty() {
        return None;
    }

    let mut sql = format!(
        "{} {}",
        quote_sqlite_identifier(name),
        create_table_sqlite_column_type(&column.data_type)
    );
    if inline_auto_increment_id && column.auto_increment {
        sql.push_str(" PRIMARY KEY AUTOINCREMENT");
    } else if !column.nullable || column.primary_key {
        sql.push_str(" NOT NULL");
    }
    if !column.default_value.trim().is_empty() {
        sql.push_str(" DEFAULT ");
        sql.push_str(column.default_value.trim());
    }
    Some(sql)
}

fn create_table_sqlite_column_type(data_type: &str) -> String {
    let base = create_table_base_type(data_type);
    match base.as_str() {
        "int" | "integer" | "tinyint" | "smallint" | "mediumint" | "bigint" | "boolean" => {
            "INTEGER".to_string()
        }
        "real" | "float" | "double" => "REAL".to_string(),
        "numeric" | "decimal" => "NUMERIC".to_string(),
        "blob" | "binary" | "varbinary" | "tinyblob" | "mediumblob" | "longblob" => {
            "BLOB".to_string()
        }
        _ => "TEXT".to_string(),
    }
}

fn create_table_design_sql_preview_for_provider(
    provider: &dyn CreateTableProvider,
    create: &CreateTableState,
) -> Result<String, String> {
    if let Some(message) = provider.validation_error(create) {
        return Err(message.to_string());
    }
    let statements = provider.design_statements(create)?;
    if statements.is_empty() {
        return Err("没有需要保存的变更".to_string());
    }
    Ok(statements.join("\n"))
}

pub fn rename_table_sql_preview(
    database_kind: DatabaseKind,
    old_name: &str,
    new_name: &str,
) -> Result<String, String> {
    let provider = table_action_sql_provider(database_kind);
    let old_name = old_name.trim();
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err("表名不能为空".to_string());
    }
    if old_name == new_name {
        return Err("表名没有变化".to_string());
    }
    provider.rename_table_sql(old_name, new_name)
}

pub fn copy_table_sql_preview(
    database_kind: DatabaseKind,
    source_name: &str,
    target_name: &str,
    copy_data: bool,
) -> Result<String, String> {
    copy_table_sql_preview_with_source_ddl(database_kind, source_name, target_name, copy_data, None)
}

pub fn copy_table_sql_preview_with_source_ddl(
    database_kind: DatabaseKind,
    source_name: &str,
    target_name: &str,
    copy_data: bool,
    source_ddl: Option<&str>,
) -> Result<String, String> {
    let provider = table_action_sql_provider(database_kind);
    let source_name = source_name.trim();
    let target_name = target_name.trim();
    if target_name.is_empty() {
        return Err("表名不能为空".to_string());
    }
    if source_name == target_name {
        return Err("新表名不能和原表相同".to_string());
    }
    provider.copy_table_sql(source_name, target_name, copy_data, source_ddl)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForeignKeyCheckMode {
    Default,
    Enable,
    Disable,
}

pub fn drop_table_sql_preview(
    database_kind: DatabaseKind,
    table_name: &str,
    foreign_key_check: ForeignKeyCheckMode,
) -> Result<String, String> {
    let provider = table_action_sql_provider(database_kind);
    let table_name = table_name.trim();
    if table_name.is_empty() {
        return Err("表名不能为空".to_string());
    }
    let sql = provider.drop_table_sql(table_name)?;
    provider.with_foreign_key_check(sql, foreign_key_check)
}

pub fn truncate_table_sql_preview(
    database_kind: DatabaseKind,
    table_name: &str,
    foreign_key_check: ForeignKeyCheckMode,
) -> Result<String, String> {
    let provider = table_action_sql_provider(database_kind);
    let table_name = table_name.trim();
    if table_name.is_empty() {
        return Err("表名不能为空".to_string());
    }
    let sql = provider.truncate_table_sql(table_name)?;
    provider.with_foreign_key_check(sql, foreign_key_check)
}

trait TableActionSqlProvider: Sync {
    fn rename_table_sql(&self, old_name: &str, new_name: &str) -> Result<String, String>;
    fn copy_table_sql(
        &self,
        source_name: &str,
        target_name: &str,
        copy_data: bool,
        source_ddl: Option<&str>,
    ) -> Result<String, String>;
    fn drop_table_sql(&self, table_name: &str) -> Result<String, String>;
    fn truncate_table_sql(&self, table_name: &str) -> Result<String, String>;

    fn with_foreign_key_check(
        &self,
        sql: String,
        foreign_key_check: ForeignKeyCheckMode,
    ) -> Result<String, String> {
        match foreign_key_check {
            ForeignKeyCheckMode::Default => Ok(sql),
            ForeignKeyCheckMode::Enable | ForeignKeyCheckMode::Disable => {
                Err("当前连接类型不支持外键检查选项".to_string())
            }
        }
    }
}

struct MySqlTableActionSqlProvider;
struct SqliteTableActionSqlProvider;
struct UnsupportedTableActionSqlProvider;

impl TableActionSqlProvider for MySqlTableActionSqlProvider {
    fn rename_table_sql(&self, old_name: &str, new_name: &str) -> Result<String, String> {
        Ok(format!(
            "ALTER TABLE {} RENAME TO {};",
            quote_mysql_identifier(old_name),
            quote_mysql_identifier(new_name)
        ))
    }

    fn copy_table_sql(
        &self,
        source_name: &str,
        target_name: &str,
        copy_data: bool,
        _: Option<&str>,
    ) -> Result<String, String> {
        let source = quote_mysql_identifier(source_name);
        let target = quote_mysql_identifier(target_name);
        let mut statements = vec![format!("CREATE TABLE {target} LIKE {source};")];
        if copy_data {
            statements.push(format!("INSERT INTO {target} SELECT * FROM {source};"));
        }
        Ok(statements.join("\n"))
    }

    fn drop_table_sql(&self, table_name: &str) -> Result<String, String> {
        Ok(format!("DROP TABLE {};", quote_mysql_identifier(table_name)))
    }

    fn truncate_table_sql(&self, table_name: &str) -> Result<String, String> {
        Ok(format!(
            "TRUNCATE TABLE {};",
            quote_mysql_identifier(table_name)
        ))
    }

    fn with_foreign_key_check(
        &self,
        sql: String,
        foreign_key_check: ForeignKeyCheckMode,
    ) -> Result<String, String> {
        match foreign_key_check {
            ForeignKeyCheckMode::Default => Ok(sql),
            ForeignKeyCheckMode::Enable => Ok(format!("SET FOREIGN_KEY_CHECKS = 1;\n{sql}")),
            ForeignKeyCheckMode::Disable => Ok(format!("SET FOREIGN_KEY_CHECKS = 0;\n{sql}")),
        }
    }
}

impl TableActionSqlProvider for SqliteTableActionSqlProvider {
    fn rename_table_sql(&self, old_name: &str, new_name: &str) -> Result<String, String> {
        Ok(format!(
            "ALTER TABLE {} RENAME TO {};",
            quote_sqlite_identifier(old_name),
            quote_sqlite_identifier(new_name)
        ))
    }

    fn copy_table_sql(
        &self,
        source_name: &str,
        target_name: &str,
        copy_data: bool,
        source_ddl: Option<&str>,
    ) -> Result<String, String> {
        let source = quote_sqlite_identifier(source_name);
        let target = quote_sqlite_identifier(target_name);
        let mut statements = vec![if let Some(source_ddl) = source_ddl {
            sqlite_copy_table_structure_sql(source_ddl, target_name)?
        } else {
            format!("CREATE TABLE {target} AS SELECT * FROM {source} WHERE 0;")
        }];
        if copy_data {
            statements.push(format!("INSERT INTO {target} SELECT * FROM {source};"));
        }
        Ok(statements.join("\n"))
    }

    fn drop_table_sql(&self, table_name: &str) -> Result<String, String> {
        Ok(format!("DROP TABLE {};", quote_sqlite_identifier(table_name)))
    }

    fn truncate_table_sql(&self, table_name: &str) -> Result<String, String> {
        Ok(format!("DELETE FROM {};", quote_sqlite_identifier(table_name)))
    }
}

impl TableActionSqlProvider for UnsupportedTableActionSqlProvider {
    fn rename_table_sql(&self, _: &str, _: &str) -> Result<String, String> {
        Err("当前连接类型暂不支持重命名表".to_string())
    }

    fn copy_table_sql(
        &self,
        _: &str,
        _: &str,
        _: bool,
        _: Option<&str>,
    ) -> Result<String, String> {
        Err("当前连接类型暂不支持复制表".to_string())
    }

    fn drop_table_sql(&self, _: &str) -> Result<String, String> {
        Err("当前连接类型暂不支持删除表".to_string())
    }

    fn truncate_table_sql(&self, _: &str) -> Result<String, String> {
        Err("当前连接类型暂不支持清空表".to_string())
    }
}

fn sqlite_copy_table_structure_sql(source_ddl: &str, target_name: &str) -> Result<String, String> {
    // ponytail: 只复制 CREATE TABLE DDL；需要完整复制二级索引/触发器时再改写 sqlite_schema 相关 DDL。
    let ddl = source_ddl.trim().trim_end_matches(';').trim();
    let lower = ddl.to_ascii_lowercase();
    let Some(mut index) = lower.find("create table") else {
        return Err("未读取到可复制的 SQLite 表结构 DDL".to_string());
    };
    index += "create table".len();
    index = skip_ascii_whitespace(ddl, index);
    for keyword in ["if", "not", "exists"] {
        if ascii_keyword_at(ddl, index, keyword) {
            index += keyword.len();
            index = skip_ascii_whitespace(ddl, index);
        }
    }
    let name_start = index;
    let name_end = sqlite_create_table_name_end(ddl, name_start)?;
    let mut sql = String::with_capacity(ddl.len() + target_name.len() + 4);
    sql.push_str(&ddl[..name_start]);
    sql.push_str(&quote_sqlite_identifier(target_name));
    sql.push_str(ddl[name_end..].trim_end());
    sql.push(';');
    Ok(sql)
}

fn skip_ascii_whitespace(value: &str, mut index: usize) -> usize {
    while value
        .as_bytes()
        .get(index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        index += 1;
    }
    index
}

fn ascii_keyword_at(value: &str, index: usize, keyword: &str) -> bool {
    value
        .get(index..index + keyword.len())
        .is_some_and(|part| part.eq_ignore_ascii_case(keyword))
        && value
            .as_bytes()
            .get(index + keyword.len())
            .is_none_or(|byte| byte.is_ascii_whitespace())
}

fn sqlite_create_table_name_end(value: &str, start: usize) -> Result<usize, String> {
    let bytes = value.as_bytes();
    let Some(first) = bytes.get(start).copied() else {
        return Err("未读取到 SQLite 表名".to_string());
    };
    match first {
        b'"' | b'\'' | b'`' => quoted_identifier_end(bytes, start, first),
        b'[' => bytes[start + 1..]
            .iter()
            .position(|byte| *byte == b']')
            .map(|offset| start + 1 + offset + 1)
            .ok_or_else(|| "SQLite 表名引用未闭合".to_string()),
        _ => {
            let end = bytes[start..]
                .iter()
                .position(|byte| byte.is_ascii_whitespace() || *byte == b'(')
                .map(|offset| start + offset)
                .unwrap_or(value.len());
            if end == start {
                Err("未读取到 SQLite 表名".to_string())
            } else {
                Ok(end)
            }
        }
    }
}

fn quoted_identifier_end(bytes: &[u8], start: usize, quote: u8) -> Result<usize, String> {
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == quote {
            if bytes.get(index + 1) == Some(&quote) {
                index += 2;
            } else {
                return Ok(index + 1);
            }
        } else {
            index += 1;
        }
    }
    Err("SQLite 表名引用未闭合".to_string())
}

static MYSQL_TABLE_ACTION_SQL_PROVIDER: MySqlTableActionSqlProvider = MySqlTableActionSqlProvider;
static SQLITE_TABLE_ACTION_SQL_PROVIDER: SqliteTableActionSqlProvider =
    SqliteTableActionSqlProvider;
static UNSUPPORTED_TABLE_ACTION_SQL_PROVIDER: UnsupportedTableActionSqlProvider =
    UnsupportedTableActionSqlProvider;

fn table_action_sql_provider(database_kind: DatabaseKind) -> &'static dyn TableActionSqlProvider {
    match database_kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => &MYSQL_TABLE_ACTION_SQL_PROVIDER,
        DatabaseKind::Sqlite => &SQLITE_TABLE_ACTION_SQL_PROVIDER,
        DatabaseKind::MongoDb | DatabaseKind::Redis => &UNSUPPORTED_TABLE_ACTION_SQL_PROVIDER,
    }
}

fn create_table_mysql_design_statements(create: &CreateTableState) -> Result<Vec<String>, String> {
    let CreateTableMode::Design {
        object, original, ..
    } = &create.mode
    else {
        return Ok(Vec::new());
    };
    let mut statements = Vec::new();
    let old_table_name = object.name.trim();
    let new_table_name = create.table_name.trim();
    if !old_table_name.eq_ignore_ascii_case(new_table_name) {
        statements.push(format!(
            "ALTER TABLE {} RENAME TO {};",
            quote_mysql_identifier(old_table_name),
            quote_mysql_identifier(new_table_name)
        ));
    }
    let table_name = quote_mysql_identifier(new_table_name);
    let old_primary_key = create_table_primary_key_names(&original.columns);
    let new_primary_key = create_table_primary_key_names(&create.columns);
    if old_primary_key != new_primary_key && !old_primary_key.is_empty() {
        statements.push(format!("ALTER TABLE {table_name} DROP PRIMARY KEY;"));
    }
    for column in &original.columns {
        if !create.columns.iter().any(|current| current.id == column.id) {
            statements.push(format!(
                "ALTER TABLE {table_name} DROP COLUMN {};",
                quote_mysql_identifier(column.name.trim())
            ));
        }
    }
    for column in &create.columns {
        let Some(line) = column.sql_line() else {
            continue;
        };
        match original.columns.iter().find(|original| original.id == column.id) {
            Some(original) if original != column => statements.push(format!(
                "ALTER TABLE {table_name} CHANGE COLUMN {} {line};",
                quote_mysql_identifier(original.name.trim())
            )),
            None => statements.push(format!("ALTER TABLE {table_name} ADD COLUMN {line};")),
            _ => {}
        }
    }

    statements.extend(create_table_mysql_drop_missing_indexes(create, original, &table_name));
    if old_primary_key != new_primary_key {
        if let Some(line) = create_table_primary_key_line(&create.columns) {
            statements.push(format!("ALTER TABLE {table_name} ADD {line};"));
        }
    }

    statements.extend(
        create_table_index_lines(&create_table_with_indexes(
            create,
            create_table_changed_items(&original.indexes, &create.indexes),
        ))?
            .into_iter()
            .map(|line| format!("ALTER TABLE {table_name} ADD {line};")),
    );
    for foreign_key in &original.foreign_keys {
        if !create
            .foreign_keys
            .iter()
            .any(|current| current.id == foreign_key.id && current == foreign_key)
            && !foreign_key.name.trim().is_empty()
        {
            statements.push(format!(
                "ALTER TABLE {table_name} DROP FOREIGN KEY {};",
                quote_mysql_identifier(foreign_key.name.trim())
            ));
        }
    }
    statements.extend(
        create_table_foreign_key_lines(
            &create_table_with_foreign_keys(
                create,
                create_table_changed_items(&original.foreign_keys, &create.foreign_keys),
            ),
            CreateTableSqlDialect::MySql,
        )?
            .into_iter()
            .map(|line| format!("ALTER TABLE {table_name} ADD {line};")),
    );
    for check in &original.checks {
        if !create
            .checks
            .iter()
            .any(|current| current.id == check.id && current == check)
            && !check.name.trim().is_empty()
        {
            statements.push(format!(
                "ALTER TABLE {table_name} DROP CHECK {};",
                quote_mysql_identifier(check.name.trim())
            ));
        }
    }
    statements.extend(
        create_table_check_lines(
            &create_table_with_checks(
                create,
                create_table_changed_items(&original.checks, &create.checks),
            ),
            CreateTableSqlDialect::MySql,
        )?
            .into_iter()
            .map(|line| format!("ALTER TABLE {table_name} ADD {line};")),
    );
    for trigger in &original.triggers {
        if !create
            .triggers
            .iter()
            .any(|current| current.id == trigger.id && current == trigger)
        {
            statements.push(format!(
                "DROP TRIGGER {};",
                quote_mysql_identifier(trigger.name.trim())
            ));
        }
    }
    statements.extend(create_table_mysql_trigger_statements(
        &create_table_with_triggers(
            create,
            create_table_changed_items(&original.triggers, &create.triggers),
        ),
    )?);
    if create.comment != original.comment {
        statements.push(format!(
            "ALTER TABLE {table_name} COMMENT={};",
            quote_mysql_string(create.comment.trim())
        ));
    }
    if create_table_options_changed(create, original) {
        let options = create_table_mysql_table_options(create);
        if !options.is_empty() {
            statements.push(format!("ALTER TABLE {table_name} {};", options.join(" ")));
        }
    }
    if create.partition_sql != original.partition_sql || create.partition_enabled != original.partition_enabled {
        let partition_sql = create_table_mysql_partition_sql(create);
        if !partition_sql.is_empty() {
            statements.push(format!("ALTER TABLE {table_name} {partition_sql};"));
        }
    }
    Ok(statements)
}

fn create_table_sqlite_design_statements(create: &CreateTableState) -> Result<Vec<String>, String> {
    let CreateTableMode::Design {
        object, original, ..
    } = &create.mode
    else {
        return Ok(Vec::new());
    };
    if create_table_sqlite_design_needs_rebuild(create, original) {
        return create_table_sqlite_rebuild_design_statements(create, original, &object.name);
    }

    let mut statements = Vec::new();
    let old_table_name = object.name.trim();
    let new_table_name = create.table_name.trim();
    if !old_table_name.eq_ignore_ascii_case(new_table_name) {
        statements.push(format!(
            "ALTER TABLE {} RENAME TO {};",
            quote_sqlite_identifier(old_table_name),
            quote_sqlite_identifier(new_table_name)
        ));
    }
    let table_name = quote_sqlite_identifier(new_table_name);
    for column in &original.columns {
        if !create.columns.iter().any(|current| current.id == column.id) {
            statements.push(format!(
                "ALTER TABLE {table_name} DROP COLUMN {};",
                quote_sqlite_identifier(column.name.trim())
            ));
        }
    }
    for column in &create.columns {
        match original.columns.iter().find(|original| original.id == column.id) {
            Some(original) if original != column => {
                if create_table_columns_equal_except_name(original, column) {
                    statements.push(format!(
                        "ALTER TABLE {table_name} RENAME COLUMN {} TO {};",
                        quote_sqlite_identifier(original.name.trim()),
                        quote_sqlite_identifier(column.name.trim())
                    ));
                } else {
                    return Err("SQLite 暂不支持通过设计表修改已有字段类型或约束".to_string());
                }
            }
            None => {
                if let Some(line) = create_table_sqlite_column_line(column, false) {
                    statements.push(format!("ALTER TABLE {table_name} ADD COLUMN {line};"));
                }
            }
            _ => {}
        }
    }
    for index in &original.indexes {
        if !create
            .indexes
            .iter()
            .any(|current| current.id == index.id && current == index)
        {
            statements.push(format!(
                "DROP INDEX {};",
                quote_sqlite_identifier(index.name.trim())
            ));
        }
    }
    statements.extend(create_table_sqlite_index_statements(
        &create_table_with_indexes(
            create,
            create_table_changed_items(&original.indexes, &create.indexes),
        ),
    )?);
    for trigger in &original.triggers {
        if !create
            .triggers
            .iter()
            .any(|current| current.id == trigger.id && current == trigger)
        {
            statements.push(format!(
                "DROP TRIGGER {};",
                quote_sqlite_identifier(trigger.name.trim())
            ));
        }
    }
    statements.extend(create_table_sqlite_trigger_statements(
        &create_table_with_triggers(
            create,
            create_table_changed_items(&original.triggers, &create.triggers),
        ),
    )?);
    Ok(statements)
}

fn create_table_sqlite_design_needs_rebuild(
    create: &CreateTableState,
    original: &CreateTableDesignSnapshot,
) -> bool {
    if original.foreign_keys != create.foreign_keys || original.checks != create.checks {
        return true;
    }

    let original_column_ids = original
        .columns
        .iter()
        .map(|column| column.id)
        .collect::<Vec<_>>();
    let current_original_column_ids = create
        .columns
        .iter()
        .filter(|column| original.columns.iter().any(|original| original.id == column.id))
        .map(|column| column.id)
        .collect::<Vec<_>>();
    if original_column_ids != current_original_column_ids {
        return true;
    }

    create.columns.iter().any(|column| {
        original
            .columns
            .iter()
            .find(|original| original.id == column.id)
            .is_some_and(|original| {
                original != column && !create_table_columns_equal_except_name(original, column)
            })
            || (!original.columns.iter().any(|original| original.id == column.id)
                && (column.primary_key || column.auto_increment))
    })
}

fn create_table_sqlite_rebuild_design_statements(
    create: &CreateTableState,
    original: &CreateTableDesignSnapshot,
    old_table_name: &str,
) -> Result<Vec<String>, String> {
    let new_table_name = create.table_name.trim();
    // ponytail: 不查库探测临时表冲突；真遇到同名表时再改成基于 sqlite_schema 生成唯一名。
    let temp_table_name = create_table_sqlite_rebuild_temp_table_name(old_table_name, new_table_name);
    let mut statements = vec![
        "PRAGMA foreign_keys = OFF;".to_string(),
        "BEGIN TRANSACTION;".to_string(),
        format!(
            "ALTER TABLE {} RENAME TO {};",
            quote_sqlite_identifier(old_table_name),
            quote_sqlite_identifier(&temp_table_name)
        ),
        create_table_sqlite_create_table_statement(create, new_table_name)?,
    ];

    let copy_columns = create_table_sqlite_rebuild_copy_columns(create, original);
    if !copy_columns.is_empty() {
        let target_columns = copy_columns
            .iter()
            .map(|(_, current)| quote_sqlite_identifier(current.name.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        let source_columns = copy_columns
            .iter()
            .map(|(original, _)| quote_sqlite_identifier(original.name.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        statements.push(format!(
            "INSERT INTO {} ({target_columns}) SELECT {source_columns} FROM {};",
            quote_sqlite_identifier(new_table_name),
            quote_sqlite_identifier(&temp_table_name)
        ));
    }

    statements.push(format!(
        "DROP TABLE {};",
        quote_sqlite_identifier(&temp_table_name)
    ));
    statements.extend(create_table_sqlite_index_statements(create)?);
    statements.extend(create_table_sqlite_trigger_statements(create)?);
    statements.push("COMMIT;".to_string());
    statements.push("PRAGMA foreign_keys = ON;".to_string());
    Ok(statements)
}

fn create_table_sqlite_rebuild_temp_table_name(old_table_name: &str, new_table_name: &str) -> String {
    let base = format!("__gdb_rebuild_{}", old_table_name.trim());
    if base.eq_ignore_ascii_case(new_table_name.trim()) {
        format!("{base}_old")
    } else {
        base
    }
}

fn create_table_sqlite_rebuild_copy_columns<'a>(
    create: &'a CreateTableState,
    original: &'a CreateTableDesignSnapshot,
) -> Vec<(&'a CreateTableColumn, &'a CreateTableColumn)> {
    create
        .columns
        .iter()
        .filter_map(|column| {
            original
                .columns
                .iter()
                .find(|original| original.id == column.id)
                .map(|original| (original, column))
        })
        .collect()
}

fn create_table_primary_key_names(columns: &[CreateTableColumn]) -> Vec<String> {
    columns
        .iter()
        .filter(|column| column.primary_key && !column.name.trim().is_empty())
        .map(|column| column.name.trim().to_ascii_lowercase())
        .collect()
}

fn create_table_columns_equal_except_name(
    left: &CreateTableColumn,
    right: &CreateTableColumn,
) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.name = String::new();
    right.name = String::new();
    left == right
}

fn create_table_changed_items<T: Clone + PartialEq>(original: &[T], current: &[T]) -> Vec<T> {
    current
        .iter()
        .filter(|item| !original.iter().any(|original| original == *item))
        .cloned()
        .collect()
}

fn create_table_with_indexes(
    create: &CreateTableState,
    indexes: Vec<CreateTableIndex>,
) -> CreateTableState {
    let mut create = create.clone();
    create.indexes = indexes;
    create
}

fn create_table_with_foreign_keys(
    create: &CreateTableState,
    foreign_keys: Vec<CreateTableForeignKey>,
) -> CreateTableState {
    let mut create = create.clone();
    create.foreign_keys = foreign_keys;
    create
}

fn create_table_with_checks(
    create: &CreateTableState,
    checks: Vec<CreateTableCheck>,
) -> CreateTableState {
    let mut create = create.clone();
    create.checks = checks;
    create
}

fn create_table_with_triggers(
    create: &CreateTableState,
    triggers: Vec<CreateTableTrigger>,
) -> CreateTableState {
    let mut create = create.clone();
    create.triggers = triggers;
    create
}

fn create_table_mysql_drop_missing_indexes(
    create: &CreateTableState,
    original: &CreateTableDesignSnapshot,
    table_name: &str,
) -> Vec<String> {
    original
        .indexes
        .iter()
        .filter(|index| {
            !create
                .indexes
                .iter()
                .any(|current| current.id == index.id && current == *index)
        })
        .map(|index| {
            format!(
                "ALTER TABLE {table_name} DROP INDEX {};",
                quote_mysql_identifier(index.name.trim())
            )
        })
        .collect()
}

fn create_table_options_changed(
    create: &CreateTableState,
    original: &CreateTableDesignSnapshot,
) -> bool {
    create.engine != original.engine
        || create.tablespace != original.tablespace
        || create.charset != original.charset
        || create.collation != original.collation
        || create.row_format != original.row_format
        || create.avg_row_length != original.avg_row_length
        || create.max_rows != original.max_rows
        || create.min_rows != original.min_rows
        || create.key_block_size != original.key_block_size
}

fn create_table_sqlite_index_statements(create: &CreateTableState) -> Result<Vec<String>, String> {
    let valid_columns = create
        .columns
        .iter()
        .map(|column| column.name.trim())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut names = BTreeSet::new();
    let mut statements = Vec::with_capacity(create.indexes.len());

    for index in &create.indexes {
        let name = index.name.trim();
        if name.is_empty() {
            return Err("Index name is required.".to_string());
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(format!("Duplicate index name: {name}."));
        }
        let index_type = create_table_normalized_index_type(&index.index_type);
        if matches!(index_type, "FULLTEXT" | "SPATIAL") {
            return Err(format!("SQLite does not support {index_type} indexes."));
        }

        let mut column_names = BTreeSet::new();
        let mut parts = Vec::with_capacity(index.columns.len());
        for column in &index.columns {
            let column_name = column.name.trim();
            if column_name.is_empty() {
                return Err(format!("Index {name} has an empty field."));
            }
            if !valid_columns.contains(&column_name.to_ascii_lowercase()) {
                return Err(format!("Index {name} references unknown field: {column_name}."));
            }
            if !column_names.insert(column_name.to_ascii_lowercase()) {
                return Err(format!("Index {name} has duplicate field: {column_name}."));
            }
            let mut part = quote_sqlite_identifier(column_name);
            let sort_order = create_table_normalized_sort_order(&column.sort_order);
            if !sort_order.is_empty() {
                part.push(' ');
                part.push_str(sort_order);
            }
            parts.push(part);
        }
        if parts.is_empty() {
            return Err(format!("Index {name} needs at least one field."));
        }

        let unique = if index_type == "UNIQUE" { "UNIQUE " } else { "" };
        statements.push(format!(
            "CREATE {unique}INDEX {} ON {} ({});",
            quote_sqlite_identifier(name),
            quote_sqlite_identifier(create.table_name.trim()),
            parts.join(", ")
        ));
    }

    Ok(statements)
}

fn create_table_mysql_validation_error(create: &CreateTableState) -> Option<&'static str> {
    if create.table_name.trim().is_empty() {
        return Some("请输入表名");
    }

    let named_columns = create
        .columns
        .iter()
        .filter(|column| !column.name.trim().is_empty())
        .collect::<Vec<_>>();
    if named_columns.is_empty() {
        return Some("请至少填写一个字段");
    }

    let mut names = BTreeSet::new();
    for column in &named_columns {
        if !names.insert(column.name.trim().to_ascii_lowercase()) {
            return Some("字段名不能重复");
        }
        if column.data_type.trim().is_empty() {
            return Some("字段类型不能为空");
        }
        if column.auto_increment
            && (!column.primary_key || !create_table_mysql_is_integer_type(&column.data_type))
        {
            return Some("自增字段必须是整数主键");
        }
        if create_table_mysql_is_text_type(&column.data_type)
            && create_table_unquoted_string_default(&column.default_value)
        {
            return Some("字符串默认值需要用引号包裹");
        }
    }

    if create
        .columns
        .iter()
        .any(|column| column.primary_key && column.name.trim().is_empty())
    {
        return Some("主键字段名不能为空");
    }

    if let Some(message) = create_table_foreign_key_validation_error(create) {
        return Some(message);
    }
    if let Some(message) = create_table_check_validation_error(create) {
        return Some(message);
    }
    if let Some(message) = create_table_trigger_validation_error(create) {
        return Some(message);
    }
    if let Some(message) = create_table_partition_validation_error(create) {
        return Some(message);
    }

    None
}

impl CreateTableIndex {
    fn new(id: u64) -> Self {
        Self {
            id,
            name: String::new(),
            columns: Vec::new(),
            index_type: String::new(),
            index_method: String::new(),
            comment: String::new(),
        }
    }
}

impl CreateTableIndexColumn {
    fn new(name: String) -> Self {
        Self {
            name,
            sub_part: String::new(),
            sort_order: String::new(),
        }
    }
}

impl CreateTableCheck {
    fn new(id: u64) -> Self {
        Self {
            id,
            name: String::new(),
            expression: String::new(),
            not_enforced: false,
        }
    }
}

impl CreateTableTrigger {
    fn new(id: u64) -> Self {
        Self {
            id,
            name: String::new(),
            timing: "BEFORE".to_string(),
            event: "INSERT".to_string(),
            body: String::new(),
        }
    }
}

impl CreateTableTriggerEvent {
    fn as_sql(self) -> &'static str {
        match self {
            CreateTableTriggerEvent::Insert => "INSERT",
            CreateTableTriggerEvent::Update => "UPDATE",
            CreateTableTriggerEvent::Delete => "DELETE",
        }
    }
}

fn create_table_check_validation_error(create: &CreateTableState) -> Option<&'static str> {
    let mut names = BTreeSet::new();
    for check in &create.checks {
        let name = check.name.trim();
        if !name.is_empty() && !names.insert(name.to_ascii_lowercase()) {
            return Some("检查名称不能重复");
        }
        if check.expression.trim().is_empty() {
            return Some("检查表达式不能为空");
        }
        if check.not_enforced
            && create.database_kind != DatabaseKind::MySql
            && create.database_kind != DatabaseKind::TiDb
        {
            return Some("当前连接类型不支持不强制实施检查");
        }
    }
    None
}

fn create_table_check_lines(
    create: &CreateTableState,
    dialect: CreateTableSqlDialect,
) -> Result<Vec<String>, String> {
    let mut names = BTreeSet::new();
    let mut lines = Vec::with_capacity(create.checks.len());
    for check in &create.checks {
        let name = check.name.trim();
        if !name.is_empty() && !names.insert(name.to_ascii_lowercase()) {
            return Err(format!("Duplicate check name: {name}."));
        }
        let expression = check.expression.trim();
        if expression.is_empty() {
            return Err("Check expression is required.".to_string());
        }

        let mut line = if name.is_empty() {
            format!("CHECK ({expression})")
        } else {
            match dialect {
                CreateTableSqlDialect::MySql => {
                    format!(
                        "CONSTRAINT {} CHECK ({expression})",
                        quote_mysql_identifier(name)
                    )
                }
                CreateTableSqlDialect::Sqlite => {
                    format!(
                        "CONSTRAINT {} CHECK ({expression})",
                        quote_sqlite_identifier(name)
                    )
                }
            }
        };
        if check.not_enforced {
            match dialect {
                CreateTableSqlDialect::MySql => line.push_str(" NOT ENFORCED"),
                CreateTableSqlDialect::Sqlite => {
                    return Err("SQLite does not support NOT ENFORCED checks.".to_string());
                }
            }
        }
        lines.push(line);
    }
    Ok(lines)
}

fn create_table_trigger_validation_error(create: &CreateTableState) -> Option<&'static str> {
    let mut names = BTreeSet::new();
    for trigger in &create.triggers {
        if trigger.name.trim().is_empty() {
            return Some("触发器名称不能为空");
        }
        if !names.insert(trigger.name.trim().to_ascii_lowercase()) {
            return Some("触发器名称不能重复");
        }
        if !matches!(
            create_table_normalized_trigger_timing(&trigger.timing),
            "BEFORE" | "AFTER"
        ) {
            return Some("触发器触发时机无效");
        }
        if !matches!(
            create_table_normalized_trigger_event(&trigger.event),
            "INSERT" | "UPDATE" | "DELETE"
        ) {
            return Some("请选择触发器事件");
        }
        if trigger.body.trim().is_empty() {
            return Some("触发器定义不能为空");
        }
    }
    None
}

fn create_table_mysql_trigger_statements(
    create: &CreateTableState,
) -> Result<Vec<String>, String> {
    create_table_trigger_statements(create, CreateTableSqlDialect::MySql)
}

fn create_table_sqlite_trigger_statements(
    create: &CreateTableState,
) -> Result<Vec<String>, String> {
    create_table_trigger_statements(create, CreateTableSqlDialect::Sqlite)
}

fn create_table_trigger_statements(
    create: &CreateTableState,
    dialect: CreateTableSqlDialect,
) -> Result<Vec<String>, String> {
    let table_name = match dialect {
        CreateTableSqlDialect::MySql => quote_mysql_identifier(create.table_name.trim()),
        CreateTableSqlDialect::Sqlite => quote_sqlite_identifier(create.table_name.trim()),
    };
    create
        .triggers
        .iter()
        .map(|trigger| {
            let name = trigger.name.trim();
            let body = trigger.body.trim();
            if name.is_empty() {
                return Err("触发器名称不能为空".to_string());
            }
            if body.is_empty() {
                return Err("触发器定义不能为空".to_string());
            }
            let trigger_name = match dialect {
                CreateTableSqlDialect::MySql => quote_mysql_identifier(name),
                CreateTableSqlDialect::Sqlite => quote_sqlite_identifier(name),
            };
            Ok(format!(
                "CREATE TRIGGER {trigger_name}\n{} {} ON {table_name}\nFOR EACH ROW\n{body};",
                create_table_normalized_trigger_timing(&trigger.timing),
                create_table_normalized_trigger_event(&trigger.event)
            ))
        })
        .collect()
}

fn create_table_primary_key_line(columns: &[CreateTableColumn]) -> Option<String> {
    let parts = columns
        .iter()
        .filter(|column| column.primary_key && !column.name.trim().is_empty())
        .map(|column| {
            let mut part = quote_mysql_identifier(column.name.trim());
            if column.supports_primary_key_prefix_length() && !column.key_length.trim().is_empty() {
                part.push('(');
                part.push_str(column.key_length.trim());
                part.push(')');
            }
            part
        })
        .collect::<Vec<_>>();

    if parts.is_empty() {
        None
    } else {
        Some(format!("PRIMARY KEY ({})", parts.join(", ")))
    }
}

fn create_table_mysql_table_options(create: &CreateTableState) -> Vec<String> {
    let mut options = Vec::new();
    if !create.engine.trim().is_empty() {
        options.push(format!("ENGINE={}", create.engine.trim()));
    }
    if !create.tablespace.trim().is_empty() {
        options.push(format!(
            "TABLESPACE {}",
            quote_mysql_identifier(create.tablespace.trim())
        ));
    }
    if !create.charset.trim().is_empty() {
        options.push(format!("DEFAULT CHARSET={}", create.charset.trim()));
    }
    if !create.collation.trim().is_empty() {
        options.push(format!("COLLATE={}", create.collation.trim()));
    }
    if !create.row_format.trim().is_empty() {
        options.push(format!("ROW_FORMAT={}", create.row_format.trim()));
    }
    create_table_push_nonzero_option(&mut options, "AVG_ROW_LENGTH", &create.avg_row_length);
    create_table_push_nonzero_option(&mut options, "MAX_ROWS", &create.max_rows);
    create_table_push_nonzero_option(&mut options, "MIN_ROWS", &create.min_rows);
    create_table_push_nonzero_option(&mut options, "KEY_BLOCK_SIZE", &create.key_block_size);
    options
}

fn create_table_push_nonzero_option(options: &mut Vec<String>, name: &str, value: &str) {
    let value = value.trim();
    if !value.is_empty() && value != "0" {
        options.push(format!("{name}={value}"));
    }
}

fn create_table_mysql_partition_sql(create: &CreateTableState) -> String {
    if !create.partition_enabled {
        return String::new();
    }
    create.partition_sql.trim().trim_end_matches(';').to_string()
}

fn create_table_partition_validation_error(create: &CreateTableState) -> Option<&'static str> {
    if !create.partition_enabled {
        return None;
    }
    if !matches!(create.database_kind, DatabaseKind::MySql | DatabaseKind::TiDb) {
        return Some("当前连接类型不支持分区");
    }
    let sql = create.partition_sql.trim();
    if sql.is_empty() {
        return Some("请填写分区定义 SQL");
    }
    if !sql.to_ascii_uppercase().starts_with("PARTITION BY ") {
        return Some("分区定义需要以 PARTITION BY 开头");
    }
    None
}

fn create_table_partition_template(method: &str, expression: &str) -> String {
    match create_table_normalized_partition_method(method) {
        "LIST" => format!("PARTITION BY LIST ({expression}) (\n  PARTITION p0 VALUES IN (0)\n)"),
        "HASH" => format!("PARTITION BY HASH ({expression})\nPARTITIONS 4"),
        "KEY" => format!("PARTITION BY KEY ({expression})\nPARTITIONS 4"),
        _ => format!("PARTITION BY RANGE ({expression}) (\n  PARTITION p0 VALUES LESS THAN (MAXVALUE)\n)"),
    }
}

fn create_table_normalized_partition_method(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "LIST" => "LIST",
        "HASH" => "HASH",
        "KEY" => "KEY",
        _ => "RANGE",
    }
}

fn create_table_digits_or_zero(value: &str) -> String {
    let digits = create_table_digits_only(value);
    if digits.is_empty() {
        "0".to_string()
    } else {
        digits
    }
}

fn create_table_normalized_table_engine(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "INNODB" => "InnoDB",
        "MYISAM" => "MyISAM",
        "MEMORY" => "MEMORY",
        "CSV" => "CSV",
        "ARCHIVE" => "ARCHIVE",
        "BLACKHOLE" => "BLACKHOLE",
        "FEDERATED" => "FEDERATED",
        _ => "",
    }
}

fn create_table_normalized_row_format(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "DEFAULT" => "DEFAULT",
        "DYNAMIC" => "DYNAMIC",
        "FIXED" => "FIXED",
        "COMPRESSED" => "COMPRESSED",
        "REDUNDANT" => "REDUNDANT",
        "COMPACT" => "COMPACT",
        _ => "",
    }
}

fn create_table_index_lines(create: &CreateTableState) -> Result<Vec<String>, String> {
    let valid_columns = create
        .columns
        .iter()
        .map(|column| column.name.trim())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut names = BTreeSet::new();
    let mut lines = Vec::with_capacity(create.indexes.len());

    for index in &create.indexes {
        let name = index.name.trim();
        if name.is_empty() {
            return Err("Index name is required.".to_string());
        }
        if name.eq_ignore_ascii_case("PRIMARY") {
            return Err("Index name cannot be PRIMARY.".to_string());
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(format!("Duplicate index name: {name}."));
        }
        if index.columns.is_empty() {
            return Err(format!("Index {name} needs at least one field."));
        }

        let mut column_names = BTreeSet::new();
        let mut parts = Vec::with_capacity(index.columns.len());
        for column in &index.columns {
            let column_name = column.name.trim();
            if column_name.is_empty() {
                return Err(format!("Index {name} has an empty field."));
            }
            if !valid_columns.contains(&column_name.to_ascii_lowercase()) {
                return Err(format!("Index {name} references unknown field: {column_name}."));
            }
            if !column_names.insert(column_name.to_ascii_lowercase()) {
                return Err(format!("Index {name} has duplicate field: {column_name}."));
            }
            if !column.sub_part.trim().chars().all(|ch| ch.is_ascii_digit()) {
                return Err(format!("Index {name} field {column_name} has invalid sub part."));
            }
            let mut part = quote_mysql_identifier(column_name);
            if !column.sub_part.trim().is_empty() {
                part.push('(');
                part.push_str(column.sub_part.trim());
                part.push(')');
            }
            let sort_order = create_table_normalized_sort_order(&column.sort_order);
            if !sort_order.is_empty() {
                part.push(' ');
                part.push_str(sort_order);
            }
            parts.push(part);
        }

        let keyword = match create_table_normalized_index_type(&index.index_type) {
            "UNIQUE" => "UNIQUE KEY",
            "FULLTEXT" => "FULLTEXT KEY",
            "SPATIAL" => "SPATIAL KEY",
            _ => "KEY",
        };
        let mut line = format!("{keyword} {}", quote_mysql_identifier(name));
        let method = create_table_normalized_index_method(&index.index_method);
        if !method.is_empty() && matches!(keyword, "KEY" | "UNIQUE KEY") {
            line.push_str(" USING ");
            line.push_str(method);
        }
        line.push_str(" (");
        line.push_str(&parts.join(", "));
        line.push(')');
        if !index.comment.trim().is_empty() {
            line.push_str(" COMMENT ");
            line.push_str(&quote_mysql_string(index.comment.trim()));
        }
        lines.push(line);
    }

    Ok(lines)
}

fn create_table_normalized_index_type(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "UNIQUE" => "UNIQUE",
        "FULLTEXT" => "FULLTEXT",
        "SPATIAL" => "SPATIAL",
        _ => "NORMAL",
    }
}

fn create_table_normalized_index_method(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "HASH" => "HASH",
        "" => "",
        _ => "BTREE",
    }
}

fn create_table_index_type_supports_method(value: &str) -> bool {
    !matches!(
        create_table_normalized_index_type(value),
        "FULLTEXT" | "SPATIAL"
    )
}

fn create_table_normalized_sort_order(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "ASC" => "ASC",
        "DESC" => "DESC",
        "" => "",
        _ => "ASC",
    }
}

fn create_table_normalized_trigger_timing(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "AFTER" => "AFTER",
        _ => "BEFORE",
    }
}

fn create_table_normalized_trigger_event(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "UPDATE" => "UPDATE",
        "DELETE" => "DELETE",
        _ => "INSERT",
    }
}

fn create_table_auto_index_name<'a>(
    table_name: &str,
    index_type: &str,
    columns: impl Iterator<Item = &'a str>,
) -> String {
    let prefix = match create_table_normalized_index_type(index_type) {
        "UNIQUE" => "uk",
        "FULLTEXT" => "ft",
        "SPATIAL" => "sp",
        _ => "idx",
    };
    let columns = columns.collect::<Vec<_>>();
    if columns.is_empty() {
        return String::new();
    }
    let mut name = format!("{prefix}_{table_name}_{}", columns.join("_"));
    if name.len() > 64 {
        name.truncate(64);
    }
    name
}

fn create_table_digits_only(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_digit()).collect()
}

fn create_table_column_type(data_type: &str, length: &str, scale: &str) -> String {
    let data_type = data_type.trim();
    if data_type.is_empty() {
        return "varchar(255)".to_string();
    }
    if !create_table_supports_length(data_type) || data_type.contains('(') {
        return data_type.to_string();
    }
    let length = length.trim();
    let scale = scale.trim();
    if !length.is_empty() && !scale.is_empty() && create_table_supports_scale(data_type) {
        return format!("{data_type}({length},{scale})");
    }
    if length.is_empty() {
        data_type.to_string()
    } else {
        format!("{data_type}({length})")
    }
}

fn create_table_split_column_type(raw_type: &str) -> (String, String, String) {
    let raw_type = raw_type.trim();
    let Some((base, rest)) = raw_type.split_once('(') else {
        return (raw_type.to_ascii_lowercase(), String::new(), String::new());
    };
    let Some((args, _)) = rest.split_once(')') else {
        return (raw_type.to_ascii_lowercase(), String::new(), String::new());
    };
    let mut parts = args.split(',').map(|part| part.trim().to_string());
    (
        base.trim().to_ascii_lowercase(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    )
}

fn create_table_supports_length(data_type: &str) -> bool {
    create_table_is_char_length_type(data_type)
        || create_table_is_binary_length_type(data_type)
        || create_table_is_decimal_type(data_type)
}

fn create_table_default_length(data_type: &str) -> Option<&'static str> {
    if create_table_is_char_length_type(data_type) || create_table_is_binary_length_type(data_type) {
        Some("255")
    } else if create_table_is_decimal_type(data_type) {
        Some("10")
    } else {
        None
    }
}

fn create_table_supports_scale(data_type: &str) -> bool {
    create_table_is_decimal_type(data_type)
}

fn create_table_is_char_length_type(data_type: &str) -> bool {
    let data_type = data_type.to_ascii_lowercase();
    data_type.contains("varchar") || data_type == "char"
}

fn create_table_is_decimal_type(data_type: &str) -> bool {
    data_type.to_ascii_lowercase().contains("decimal")
}

fn create_table_is_text_type(data_type: &str) -> bool {
    let data_type = create_table_base_type(data_type);
    matches!(data_type.as_str(), "char" | "varchar") || data_type.ends_with("text")
}

fn create_table_is_binary_length_type(data_type: &str) -> bool {
    let data_type = create_table_base_type(data_type);
    matches!(data_type.as_str(), "binary" | "varbinary")
}

fn create_table_supports_key_length(data_type: &str) -> bool {
    let data_type = create_table_base_type(data_type);
    matches!(data_type.as_str(), "char" | "varchar" | "binary" | "varbinary")
        || data_type.ends_with("text")
        || data_type.ends_with("blob")
}

fn create_table_base_type(data_type: &str) -> String {
    data_type
        .trim()
        .split_once('(')
        .map_or(data_type.trim(), |(base, _)| base.trim())
        .to_ascii_lowercase()
}

fn create_table_is_number_type(data_type: &str) -> bool {
    [
        "tinyint",
        "smallint",
        "mediumint",
        "int",
        "integer",
        "bigint",
        "float",
        "double",
        "decimal",
    ]
    .iter()
    .any(|value| data_type.to_ascii_lowercase().contains(value))
}

fn create_table_is_integer_type(data_type: &str) -> bool {
    matches!(
        create_table_base_type(data_type).as_str(),
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint"
    )
}

fn create_table_unquoted_string_default(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !value.eq_ignore_ascii_case("NULL")
        && !((value.starts_with('\'') && value.ends_with('\''))
            || (value.starts_with('"') && value.ends_with('"')))
}

fn create_table_is_auto_update_time_type(data_type: &str) -> bool {
    let data_type = data_type.to_ascii_lowercase();
    data_type.contains("datetime") || data_type.contains("timestamp")
}

fn quote_mysql_identifier(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}

fn quote_mysql_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}

fn quote_sqlite_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn create_table_mysql_supports_length(data_type: &str) -> bool {
    create_table_supports_length(data_type)
}

fn create_table_mysql_default_length(data_type: &str) -> Option<&'static str> {
    create_table_default_length(data_type)
}

fn create_table_mysql_supports_scale(data_type: &str) -> bool {
    create_table_supports_scale(data_type)
}

fn create_table_mysql_is_text_type(data_type: &str) -> bool {
    create_table_is_text_type(data_type)
}

fn create_table_mysql_supports_key_length(data_type: &str) -> bool {
    create_table_supports_key_length(data_type)
}

fn create_table_mysql_is_number_type(data_type: &str) -> bool {
    create_table_is_number_type(data_type)
}

fn create_table_mysql_is_integer_type(data_type: &str) -> bool {
    create_table_is_integer_type(data_type)
}

fn create_table_mysql_is_auto_update_time_type(data_type: &str) -> bool {
    create_table_is_auto_update_time_type(data_type)
}
