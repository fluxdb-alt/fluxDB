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

