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
    if matches!(create.database_kind, DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb) {
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

/// 设计表加载的类型名 → 编辑器基础类型 + 长度/精度。
///
/// 驱动回传的类型名可能带方言修饰（PG 的 `character varying(255)`、
/// `timestamp without time zone`，MySQL 的 `int unsigned`）；这里统一成下拉选项，
/// 无法识别的类型整体保留，由界面按自定义类型展示。
fn create_table_design_type_parts(
    database_kind: DatabaseKind,
    provider: &dyn CreateTableProvider,
    raw_type: &str,
) -> (String, String, String) {
    let raw_type = provider.normalize_metadata_type(raw_type);
    let raw_type = raw_type.as_str();
    match database_kind {
        DatabaseKind::Postgres => create_table_postgres_design_type_parts(raw_type),
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => create_table_mysql_design_type_parts(raw_type),
        DatabaseKind::Sqlite => create_table_sqlite_design_type_parts(raw_type),
        _ => create_table_split_column_type(raw_type),
    }
}

fn create_table_sqlite_design_type_parts(raw_type: &str) -> (String, String, String) {
    let (base, length, scale) = create_table_split_column_type(raw_type);
    if base.contains('(') {
        return (base, length, scale);
    }
    if create_table_sqlite_supports_length(&base) || create_table_sqlite_supports_scale(&base) {
        (base, length, scale)
    } else {
        (raw_type.trim().to_string(), String::new(), String::new())
    }
}

fn create_table_mysql_design_type_parts(raw_type: &str) -> (String, String, String) {
    let (base, length, scale) = create_table_split_column_type(raw_type);
    // enum/set 等非数字参数已由 split 整体保留。
    if base.contains('(') {
        return (base, length, scale);
    }
    // 只有已知支持长度/精度的类型才拆数字参数；BIT(8)、VECTOR(1536) 等
    // 数字参数不是列长度，拆开后在回写时会被静默丢掉。
    if create_table_mysql_supports_length(&base) || create_table_mysql_supports_scale(&base) {
        (base, length, scale)
    } else {
        (raw_type.trim().to_string(), String::new(), String::new())
    }
}

fn create_table_mysql_type_values(raw_type: &str) -> Option<(String, Vec<String>)> {
    let raw_type = raw_type.trim();
    let (base, rest) = raw_type.split_once('(')?;
    let base = base.trim().to_ascii_lowercase();
    if !matches!(base.as_str(), "enum" | "set") {
        return None;
    }
    let args = rest.strip_suffix(')')?.trim();
    let mut values = Vec::new();
    let mut chars = args.chars().peekable();
    loop {
        while chars.next_if(|ch| ch.is_whitespace() || *ch == ',').is_some() {}
        let Some(quote) = chars.next() else {
            break;
        };
        if quote != '\'' && quote != '"' {
            return None;
        }
        let mut value = String::new();
        let mut closed = false;
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                value.push(chars.next()?);
            } else if ch == quote {
                if chars.peek() == Some(&quote) {
                    chars.next();
                    value.push(quote);
                } else {
                    closed = true;
                    break;
                }
            } else {
                value.push(ch);
            }
        }
        if !closed {
            return None;
        }
        values.push(value);
    }
    Some((base, values))
}

fn create_table_postgres_design_type_parts(raw_type: &str) -> (String, String, String) {
    let raw_type = raw_type.trim();
    let raw = raw_type.to_ascii_lowercase();
    // 数组、带引号或 schema 限定的自定义类型没有可拆分的列长度语义，整体保留。
    if raw.contains('[') || raw.contains('"') || raw.contains('.') {
        return (raw_type.to_string(), String::new(), String::new());
    }
    let (base, rest) = match raw.split_once('(') {
        Some((base, rest)) => (base.trim(), Some(rest)),
        None => (raw.as_str(), None),
    };
    let (args, suffix) = rest
        .and_then(|rest| rest.split_once(')'))
        .map(|(args, suffix)| (args.trim(), suffix.trim()))
        .unwrap_or(("", ""));
    // 时区修饰在括号后（`timestamp(6) with time zone`），需要和后缀一起判断。
    let canonical = match (base, suffix) {
        ("character varying", "") => "varchar",
        ("character", "") => "char",
        ("varchar", "") => "varchar",
        ("char", "") => "char",
        ("timestamp without time zone", "") | ("timestamp", "") => "timestamp",
        ("timestamp with time zone", "") | ("timestamp", "with time zone") => "timestamptz",
        ("timestamptz", "") => "timestamptz",
        ("timestamp", "without time zone") => "timestamp",
        ("time without time zone", "") | ("time", "") => "time",
        ("time with time zone", "") | ("time", "with time zone") => "timetz",
        ("timetz", "") => "timetz",
        ("time", "without time zone") => "time",
        ("bit varying", "") => "varbit",
        ("bit", "") => "bit",
        ("varbit", "") => "varbit",
        ("numeric" | "decimal", "") => base,
        // 未知类型或未知修饰（含参数）整体保留，避免保存时丢定义。
        _ => return (raw_type.to_string(), String::new(), String::new()),
    };
    let mut parts = args.split(',').map(str::trim);
    let length = parts.next().unwrap_or_default().to_string();
    let scale = parts.next().unwrap_or_default().to_string();
    let numeric = |value: &str| value.chars().all(|ch| ch.is_ascii_digit());
    if (!length.is_empty() && !numeric(&length)) || (!scale.is_empty() && !numeric(&scale)) {
        return (raw_type.to_string(), String::new(), String::new());
    }
    (canonical.to_string(), length, scale)
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
            enum_values: Vec::new(),
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
        database_kind: DatabaseKind,
        provider: &dyn CreateTableProvider,
    ) -> Self {
        let raw_type = column
            .type_name
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| provider.default_column_type());
        let (mut data_type, mut length, mut scale) =
            create_table_design_type_parts(database_kind, provider, raw_type);
        let enum_values = if matches!(database_kind, DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb) {
            create_table_mysql_type_values(raw_type)
        } else {
            None
        };
        if let Some((base, _)) = &enum_values {
            data_type = base.clone();
            length.clear();
            scale.clear();
        }
        let lower_raw_type = raw_type.to_ascii_lowercase();
        Self {
            id,
            name: column.name,
            data_type,
            enum_values: enum_values.map(|(_, values)| values).unwrap_or_default(),
            length,
            scale,
            nullable: column.nullable,
            primary_key: column.primary_key,
            default_value: String::new(),
            comment: column.comment.unwrap_or_default(),
            auto_increment: false,
            auto_update_time: false,
            unsigned: lower_raw_type.contains(" unsigned"),
            zerofill: lower_raw_type.contains(" zerofill"),
            binary: false,
            charset: String::new(),
            collation: String::new(),
            key_length: String::new(),
        }
    }

    fn set_data_type(&mut self, provider: &dyn CreateTableProvider, data_type: String) {
        let old_default = provider.default_length(&self.data_type);
        let old_capabilities = provider.type_capabilities(&self.data_type);
        let capabilities = provider.type_capabilities(&data_type);
        let is_temporal_precision = |data_type: &str| {
            matches!(
                create_table_base_type(data_type).as_str(),
                "datetime" | "timestamp" | "timestamptz" | "time" | "timetz"
            )
        };
        // 字符长度、数值精度和小数秒精度不能互相沿用；同族切换保留用户参数。
        if old_capabilities.length != capabilities.length
            || old_capabilities.bit_length != capabilities.bit_length
            || old_capabilities.scale != capabilities.scale
            || is_temporal_precision(&self.data_type) != is_temporal_precision(&data_type)
        {
            self.length.clear();
            self.scale.clear();
        }
        let old_length = self.length.trim().to_string();
        let old_base = create_table_base_type(&self.data_type);
        self.data_type = data_type;
        let old_enum = matches!(old_base.as_str(), "enum" | "set");
        let new_base = create_table_base_type(&self.data_type);
        if matches!(new_base.as_str(), "enum" | "set") {
            if !old_enum {
                self.enum_values = vec![String::new()];
            }
        } else {
            self.enum_values.clear();
        }

        if capabilities.length {
            if let Some(default_length) = provider.default_length(&self.data_type)
                && (old_length.is_empty() || Some(old_length.as_str()) == old_default)
            {
                self.length = default_length.to_string();
            }
        } else {
            self.length.clear();
        }


        if !capabilities.scale {
            self.scale.clear();
        }
        if !capabilities.key_length {
            self.key_length.clear();
        }
        if !capabilities.text_options {
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
            fluxdb_core::MySqlDialect.quote_identifier(name),
            if matches!(
                create_table_base_type(&self.data_type).as_str(),
                "enum" | "set"
            ) {
                create_table_enum_type(&self.data_type, &self.enum_values)
            } else {
                create_table_column_type(&self.data_type, &self.length, &self.scale)
            }
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

#[cfg(test)]
mod create_table_design_type_tests {
    use super::*;

    fn completion(name: &str, type_name: &str) -> CompletionColumn {
        CompletionColumn {
            database: Some("db".into()),
            schema: Some("public".into()),
            table: "items".into(),
            name: name.into(),
            type_name: Some(type_name.into()),
            nullable: true,
            primary_key: false,
            comment: None,
            stable: None,
        }
    }

    fn design(kind: DatabaseKind, columns: Vec<CompletionColumn>) -> CreateTableState {
        CreateTableState::design(
            ObjectPath {
                connection_id: ConnectionId(1),
                database: Some("db".into()),
                schema: Some("public".into()),
                name: "items".into(),
                kind: ObjectKind::Table,
            },
            kind,
            columns,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )
    }

    #[test]
    fn postgres_catalog_types_use_editor_names() {
        let create = design(
            DatabaseKind::Postgres,
            vec![
                completion("name", "character varying(64)"),
                completion("code", "character(8)"),
                completion("created_at", "timestamp without time zone"),
                completion("updated_at", "timestamp(6) with time zone"),
                completion("open_at", "time with time zone"),
                completion("close_at", "time(6) without time zone"),
                completion("amount", "numeric(12,4)"),
                completion("flags", "bit varying(8)"),
                completion("precise_at", "timestamp(6) without time zone"),
            ],
        );
        let columns = &create.columns;
        assert_eq!(
            (columns[0].data_type.as_str(), columns[0].length.as_str()),
            ("varchar", "64")
        );
        assert_eq!(
            (columns[1].data_type.as_str(), columns[1].length.as_str()),
            ("char", "8")
        );
        assert_eq!(
            (columns[2].data_type.as_str(), columns[2].length.as_str()),
            ("timestamp", "")
        );
        assert_eq!(
            (columns[3].data_type.as_str(), columns[3].length.as_str()),
            ("timestamptz", "6")
        );
        assert_eq!(
            (columns[4].data_type.as_str(), columns[4].length.as_str()),
            ("timetz", "")
        );
        assert_eq!(
            (columns[5].data_type.as_str(), columns[5].length.as_str()),
            ("time", "6")
        );
        assert_eq!(
            (
                columns[6].data_type.as_str(),
                columns[6].length.as_str(),
                columns[6].scale.as_str(),
            ),
            ("numeric", "12", "4")
        );
        assert_eq!(
            (columns[7].data_type.as_str(), columns[7].length.as_str()),
            ("varbit", "8")
        );
        assert_eq!(
            (columns[8].data_type.as_str(), columns[8].length.as_str()),
            ("timestamp", "6")
        );
        assert_eq!(
            create_table_postgres_column_type(&columns[3]),
            "timestamptz(6)"
        );
        assert_eq!(create_table_postgres_column_type(&columns[5]), "time(6)");
        assert_eq!(
            create_table_postgres_column_type(&columns[8]),
            "timestamp(6)"
        );
        assert!(create.type_capabilities("varchar").length);
        assert!(create.type_capabilities("char").length);
        assert!(create.type_capabilities("numeric").length);
        assert!(create.type_capabilities("numeric").scale);
        assert!(create.type_capabilities("timestamp").length);
        assert!(create.type_capabilities("timestamptz").length);
        assert!(create.type_capabilities("time").length);
        assert!(create.type_capabilities("timetz").length);
    }

    #[test]
    fn postgres_ambiguous_or_custom_types_stay_intact() {
        let provider = create_table_provider(DatabaseKind::Postgres);
        for raw_type in [
            "interval(3)",
            "character varying(32)[]",
            "numeric(12,4)[]",
            "timestamp(6) with time zone[]",
            "integer[]",
            "public.order_state",
            r#"public."OrderState""#,
            r#""Type(WithParams)""#,
            "numeric(12,-2)",
        ] {
            let mut create = design(
                DatabaseKind::Postgres,
                vec![completion("value", raw_type)],
            );
            assert_eq!(create.columns[0].data_type, raw_type);
            let capabilities = create.type_capabilities(raw_type);
            assert!(!capabilities.length);
            assert!(!capabilities.bit_length);
            assert!(!capabilities.scale);
            assert_eq!(
                create_table_postgres_column_type(&create.columns[0]),
                raw_type,
                "DDL must preserve the complete type: {raw_type}"
            );

            create.set_column_field(1, CreateTableColumnField::Length, "8".into());
            create.set_column_field(1, CreateTableColumnField::Scale, "2".into());
            assert!(create.columns[0].length.is_empty());
            assert!(create.columns[0].scale.is_empty());
            assert_eq!(provider.design_statements(&create), Ok(Vec::new()));

            create.set_column_field(1, CreateTableColumnField::Comment, "kept".into());
            assert_eq!(
                create.sql_preview(),
                Ok(r#"COMMENT ON COLUMN "public"."items"."value" IS 'kept';"#.into()),
                "comment edit must not change the column type: {raw_type}"
            );
            create.mode = CreateTableMode::Create;
            assert!(
                create.sql_preview().expect("create preview")
                    .contains(&format!(r#""value" {raw_type}"#)),
                "CREATE must retain the complete type: {raw_type}"
            );
        }
    }

    #[test]
    fn postgres_type_changes_distinguish_opaque_and_scalar_definitions() {
        for (raw_type, target_type) in [
            ("numeric(12,4)[]", "numeric"),
            ("timestamp(6) with time zone[]", "timestamp"),
            ("interval(3)", "interval"),
        ] {
            let mut create = design(
                DatabaseKind::Postgres,
                vec![completion("value", raw_type)],
            );
            create.set_column_field(1, CreateTableColumnField::DataType, target_type.into());
            assert_eq!(
                create.sql_preview(),
                Ok(format!(
                    r#"ALTER TABLE "public"."items" ALTER COLUMN "value" TYPE {target_type};"#
                )),
                "an intentional type change must not disappear: {raw_type}"
            );
        }
    }

    #[test]
    fn type_switches_reset_incompatible_parameters_and_keep_same_family() {
        for (kind, raw_type, target, length, scale) in [
            (DatabaseKind::Postgres, "character varying(64)", "timestamp", "", ""),
            (DatabaseKind::Postgres, "numeric(12,4)", "time", "", ""),
            (DatabaseKind::Postgres, "timestamp(6) without time zone", "numeric", "", ""),
            (DatabaseKind::Postgres, "numeric(12,4)", "varchar", "", ""),
            (DatabaseKind::Postgres, "numeric(12,4)", "decimal", "12", "4"),
            (DatabaseKind::Postgres, "timestamp(6) without time zone", "timetz", "6", ""),
            (DatabaseKind::Postgres, "character varying(64)", "char", "64", ""),
            (DatabaseKind::MySql, "varchar(64)", "datetime", "", ""),
            (DatabaseKind::MySql, "decimal(12,4)", "timestamp", "", ""),
            (DatabaseKind::MySql, "timestamp(6)", "decimal", "10", ""),
            (DatabaseKind::MySql, "decimal(12,4)", "varchar", "255", ""),
            (DatabaseKind::MySql, "int(10)", "varchar", "255", ""),
            (DatabaseKind::MySql, "timestamp(6)", "time", "6", ""),
            (DatabaseKind::TiDb, "varchar(64)", "timestamp", "", ""),
            (DatabaseKind::Dameng, "varchar(64)", "number", "10", ""),
            (DatabaseKind::Dameng, "number(12,4)", "varchar", "255", ""),
        ] {
            let mut create = design(kind, vec![completion("value", raw_type)]);
            create.set_column_field(1, CreateTableColumnField::DataType, target.into());
            assert_eq!(
                (create.columns[0].length.as_str(), create.columns[0].scale.as_str()),
                (length, scale),
                "{kind:?}: {raw_type} -> {target}"
            );
        }
    }

    #[test]
    fn mysql_catalog_types_drop_unsigned_and_zerofill_suffixes() {
        let create = design(
            DatabaseKind::MySql,
            vec![
                completion("id", "int unsigned"),
                completion("score", "int(10) unsigned zerofill"),
                completion("name", "varchar(64)"),
                completion("status", "enum('A','B')"),
                completion("created_at", "timestamp(6)"),
                completion("open_at", "time(3)"),
            ],
        );
        let columns = &create.columns;
        assert_eq!(columns[0].data_type, "int");
        assert!(columns[0].unsigned);
        assert_eq!(columns[1].data_type, "int(10)");
        assert!(columns[1].unsigned);
        assert!(columns[1].zerofill);
        assert_eq!(
            columns[1].sql_line().as_deref(),
            Some("`score` int(10) UNSIGNED ZEROFILL")
        );
        assert_eq!(
            (columns[2].data_type.as_str(), columns[2].length.as_str()),
            ("varchar", "64")
        );
        assert_eq!(
            (columns[4].data_type.as_str(), columns[4].length.as_str()),
            ("timestamp", "6")
        );
        assert_eq!(
            (columns[5].data_type.as_str(), columns[5].length.as_str()),
            ("time", "3")
        );
        assert!(create.type_capabilities("timestamp").length);
        assert!(create.type_capabilities("time").length);
        assert_eq!(
            columns[4].sql_line().as_deref(),
            Some("`created_at` timestamp(6)")
        );
        assert_eq!(columns[5].sql_line().as_deref(), Some("`open_at` time(3)"));
        assert_eq!(columns[3].data_type, "enum");
        assert_eq!(columns[3].enum_values, ["A", "B"]);
        assert!(columns[3].length.is_empty());
        assert_eq!(
            columns[3].sql_line().as_deref(),
            Some("`status` enum('A','B')")
        );
    }

    #[test]
    fn mysql_enum_and_set_values_are_structured_and_round_trip() {
        let create = design(
            DatabaseKind::MySql,
            vec![
                completion("status", r#"enum('New','Done''Today','A\\B')"#),
                completion("tags", "set('a','b')"),
            ],
        );
        let columns = &create.columns;
        assert_eq!(columns[0].data_type, "enum");
        assert_eq!(columns[0].enum_values, ["New", "Done'Today", r#"A\B"#]);
        assert_eq!(
            columns[0].sql_line().as_deref(),
            Some(r#"`status` enum('New','Done''Today','A\\B')"#)
        );
        assert_eq!(columns[1].data_type, "set");
        assert_eq!(columns[1].enum_values, ["a", "b"]);
        assert_eq!(columns[1].sql_line().as_deref(), Some("`tags` set('a','b')"));
    }

    #[test]
    fn mysql_enum_and_set_values_are_validated() {
        let mut create = design(
            DatabaseKind::MySql,
            vec![completion("status", "enum('A','B')")],
        );
        create.table_name = "items".into();
        create.mode = CreateTableMode::Create;
        assert_eq!(create.validation_error(), None);

        create.columns[0].enum_values.clear();
        assert_eq!(
            create.validation_error(),
            Some("ENUM/SET 至少需要一个成员")
        );

        create.columns[0].enum_values = vec!["A".into(), "".into()];
        assert_eq!(
            create.validation_error(),
            Some("ENUM/SET 成员不能为空")
        );

        create.columns[0].enum_values = vec!["A".into(), "A".into()];
        assert_eq!(
            create.validation_error(),
            Some("ENUM/SET 成员不能重复")
        );

        create.columns[0].data_type = "set".into();
        create.columns[0].enum_values = (0..65).map(|index| format!("v{index}")).collect();
        assert_eq!(create.validation_error(), Some("SET 最多支持 64 个成员"));
    }

    #[test]
    fn mysql_unsupported_numeric_parameters_stay_opaque() {
        let raw_type = "vector(1536)";
        let mut create = design(
            DatabaseKind::MySql,
            vec![completion("value", raw_type)],
        );
        let column = &create.columns[0];
        assert_eq!(column.data_type, raw_type);
        assert!(column.length.is_empty());
        assert!(column.scale.is_empty());
        assert!(!create.type_capabilities(raw_type).length);
        assert!(
            column
                .sql_line()
                .is_some_and(|line| line.contains(raw_type))
        );

        create.set_column_field(1, CreateTableColumnField::Comment, "kept".into());
        assert!(
            create
                .sql_preview()
                .is_ok_and(|sql| sql.contains(raw_type)),
            "comment edit must preserve {raw_type}"
        );

        let create = design(
            DatabaseKind::MySql,
            vec![completion("ratio", "float(7,4)")],
        );
        assert_eq!(
            (
                create.columns[0].data_type.as_str(),
                create.columns[0].length.as_str(),
                create.columns[0].scale.as_str(),
            ),
            ("float", "7", "4")
        );
        assert_eq!(
            create.columns[0].sql_line().as_deref(),
            Some("`ratio` float(7,4)")
        );

        let create = design(
            DatabaseKind::MySql,
            vec![completion("bits", "bit(8)")],
        );
        assert_eq!(
            (create.columns[0].data_type.as_str(), create.columns[0].length.as_str()),
            ("bit", "8")
        );
        assert_eq!(
            create.columns[0].sql_line().as_deref(),
            Some("`bits` bit(8)")
        );
    }

    #[test]
    fn temporal_precision_rejects_values_above_six() {
        for kind in [DatabaseKind::Postgres, DatabaseKind::MySql] {
            let mut create = design(kind, vec![completion("value", "timestamp")]);
            create.table_name = "items".into();

            create.set_column_field(1, CreateTableColumnField::Length, "7".into());
            assert_eq!(
                create.validation_error(),
                Some("时间类型精度必须在 0 到 6 之间"),
                "{kind:?}"
            );

            create.set_column_field(1, CreateTableColumnField::Length, "6".into());
            assert_eq!(create.validation_error(), None, "{kind:?}");
        }
    }

    #[test]
    fn mysql_bit_and_year_keep_their_parameter_semantics() {
        let mut create = design(
            DatabaseKind::MySql,
            vec![
                completion("flags", "bit(8)"),
                completion("year_short", "year(2)"),
                completion("year_default", "year"),
            ],
        );
        create.table_name = "items".into();
        assert_eq!(create.columns[0].data_type, "bit");
        assert_eq!(create.columns[0].length, "8");
        assert_eq!(create.columns[1].data_type, "year(2)");
        assert!(create.columns[1].length.is_empty());
        assert_eq!(create.columns[2].data_type, "year");
        assert_eq!(create.columns[2].sql_line().as_deref(), Some("`year_default` year"));

        create.set_column_field(1, CreateTableColumnField::Length, "65".into());
        assert_eq!(
            create.validation_error(),
            Some("BIT 位宽必须在 1 到 64 之间")
        );
        create.set_column_field(1, CreateTableColumnField::Length, "64".into());
        assert_eq!(create.validation_error(), None);

        assert!(!create.type_capabilities("year(2)").length);
    }

    #[test]
    fn custom_types_round_trip_in_every_supported_dialect() {
        for (kind, raw_type, expected) in [
            (DatabaseKind::MySql, "int(10) unsigned zerofill", "int(10)"),
            (DatabaseKind::MySql, "vector(1536)", "vector(1536)"),
            (DatabaseKind::Postgres, "public.\"OrderState\"", "public.\"OrderState\""),
            (DatabaseKind::Sqlite, "datetime", "datetime"),
        ] {
            let create = design(kind, vec![completion("value", raw_type)]);
            assert_eq!(create.columns[0].data_type, expected, "{kind:?}: {raw_type}");
        }
    }

    #[test]
    fn sqlite_preserves_declared_type_parameters() {
        for raw_type in ["varchar(64)", "decimal(10,2)", "custom_type"] {
            let mut create = design(
                DatabaseKind::Sqlite,
                vec![completion("value", raw_type)],
            );
            create.table_name = "items".into();
            create.mode = CreateTableMode::Create;
            let expected_base = raw_type
                .split_once('(')
                .map_or(raw_type, |(base, _)| base);
            assert_eq!(create.columns[0].data_type, expected_base);
            assert!(
                create
                    .sql_preview()
                    .is_ok_and(|sql| sql.contains(raw_type)),
                "SQLite must preserve {raw_type}"
            );
        }
    }

    #[test]
    fn sqlite_common_types_expose_length_and_scale() {
        let create = design(
            DatabaseKind::Sqlite,
            vec![
                completion("name", "varchar(64)"),
                completion("amount", "decimal(10,2)"),
            ],
        );
        assert_eq!(
            (
                create.columns[0].data_type.as_str(),
                create.columns[0].length.as_str(),
                create.columns[0].scale.as_str(),
            ),
            ("varchar", "64", "")
        );
        assert_eq!(
            (
                create.columns[1].data_type.as_str(),
                create.columns[1].length.as_str(),
                create.columns[1].scale.as_str(),
            ),
            ("decimal", "10", "2")
        );
        assert!(create.type_capabilities("varchar").length);
        assert!(create.type_capabilities("decimal").scale);
    }

    #[test]
    fn postgres_bit_precision_is_not_limited_by_mysql_range() {
        let mut create = design(
            DatabaseKind::Postgres,
            vec![completion("flags", "bit(100)")],
        );
        create.table_name = "items".into();
        create.mode = CreateTableMode::Create;
        assert_eq!(create.validation_error(), None);
        assert_eq!(
            create_table_postgres_column_type(&create.columns[0]),
            "bit(100)"
        );
    }

    #[test]
    fn dialect_type_options_cover_core_and_extension_gaps() {
        let mysql = create_table_provider(DatabaseKind::MySql).type_options();
        for expected in [
            "bit", "year", "enum", "set", "geometry", "point", "vector",
        ] {
            assert!(mysql.contains(&expected), "MySQL missing {expected}");
        }
        let postgres = create_table_provider(DatabaseKind::Postgres).type_options();
        for expected in [
            "bit", "varbit", "smallserial", "int2", "float8", "tsvector", "int4range",
            "citext",
        ] {
            assert!(postgres.contains(&expected), "PostgreSQL missing {expected}");
        }
        let sqlite = create_table_provider(DatabaseKind::Sqlite).type_options();
        for expected in ["any", "char", "varchar", "clob", "enum", "set"] {
            assert!(sqlite.contains(&expected), "SQLite missing {expected}");
        }
    }

    #[test]
    fn new_mysql_types_survive_create_preview() {
        let mut create = design(
            DatabaseKind::MySql,
            vec![
                completion("flags", "bit(8)"),
                completion("year_short", "year(2)"),
                completion("status", "enum('New','Done')"),
                completion("tags", "set('a','b')"),
                completion("shape", "geometry"),
                completion("embedding", "vector(1536)"),
            ],
        );
        create.table_name = "items".into();
        create.mode = CreateTableMode::Create;
        let sql = create.sql_preview().expect("MySQL create preview");
        for expected in [
            "bit(8)",
            "year(2)",
            "enum('New','Done')",
            "set('a','b')",
            "geometry",
            "vector(1536)",
        ] {
            assert!(sql.contains(expected), "missing {expected}:\n{sql}");
        }
    }

    #[test]
    fn new_postgres_types_survive_create_preview() {
        let mut create = design(
            DatabaseKind::Postgres,
            vec![
                completion("small_id", "smallserial"),
                completion("flags", "bit(8)"),
                completion("varying_flags", "varbit(16)"),
                completion("search", "tsvector"),
                completion("bounds", "int4range"),
                completion("label", "citext"),
            ],
        );
        create.table_name = "items".into();
        create.mode = CreateTableMode::Create;
        let sql = create.sql_preview().expect("PostgreSQL create preview");
        for expected in [
            "smallserial",
            "bit(8)",
            "varbit(16)",
            "tsvector",
            "int4range",
            "citext",
        ] {
            assert!(sql.contains(expected), "missing {expected}:\n{sql}");
        }
    }
}
