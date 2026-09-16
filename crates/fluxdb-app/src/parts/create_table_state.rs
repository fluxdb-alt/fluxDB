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
            schema: String::new(),
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
        // 设计模式必须带上原表 schema：PG 下同名跨 schema 的设计/保存否则会落到别的 schema。
        create.schema = object.schema.clone().unwrap_or_default();
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
