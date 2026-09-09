#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CreateTableSqlDialect {
    MySql,
    Sqlite,
}

fn create_table_add_foreign_key(create: &mut CreateTableState) {
    let id = create.next_foreign_key_id;
    create.next_foreign_key_id += 1;
    let mut foreign_key = CreateTableForeignKey::new(id, create.database.clone());
    if let Some(column) = create
        .columns
        .iter()
        .map(|column| column.name.trim())
        .find(|name| !name.is_empty())
    {
        foreign_key.columns.push(column.to_string());
        foreign_key.referenced_columns.push(column.to_string());
    }
    create.foreign_keys.push(foreign_key);
    create.selected_foreign_key_id = Some(id);
    create.autofill_empty_foreign_key_names();
}

fn create_table_set_foreign_key_field(
    create: &mut CreateTableState,
    foreign_key_id: u64,
    field: CreateTableForeignKeyField,
    value: String,
) {
    let Some(foreign_key) = create
        .foreign_keys
        .iter_mut()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
    else {
        return;
    };
    match field {
        CreateTableForeignKeyField::Name => foreign_key.name = value,
        CreateTableForeignKeyField::ReferencedDatabase => {
            if foreign_key.referenced_database != value {
                foreign_key.referenced_database = value;
                foreign_key.referenced_table.clear();
                foreign_key.referenced_columns.clear();
                foreign_key.referenced_column_options = LoadState::NotLoaded;
            }
        }
        CreateTableForeignKeyField::ReferencedTable => {
            if foreign_key.referenced_table != value {
                foreign_key.referenced_table = value;
                foreign_key.referenced_columns.clear();
                foreign_key.referenced_column_options = LoadState::NotLoaded;
            }
        }
        CreateTableForeignKeyField::OnDelete => {
            foreign_key.on_delete = create_table_normalized_foreign_key_action(&value).to_string();
        }
        CreateTableForeignKeyField::OnUpdate => {
            foreign_key.on_update = create_table_normalized_foreign_key_action(&value).to_string();
        }
    }
    create.autofill_empty_foreign_key_names();
}

fn create_table_add_foreign_key_column(create: &mut CreateTableState, foreign_key_id: u64) {
    let used = create
        .foreign_keys
        .iter()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
        .map(|foreign_key| {
            foreign_key
                .columns
                .iter()
                .map(|column| column.to_ascii_lowercase())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let next_column = create
        .columns
        .iter()
        .map(|column| column.name.trim())
        .find(|name| !name.is_empty() && !used.contains(&name.to_ascii_lowercase()))
        .map(str::to_string)
        .unwrap_or_default();
    let Some(foreign_key) = create
        .foreign_keys
        .iter_mut()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
    else {
        return;
    };
    foreign_key.columns.push(next_column);
    create.selected_foreign_key_id = Some(foreign_key_id);
    create.autofill_empty_foreign_key_names();
}

fn create_table_move_foreign_key_column_up(
    create: &mut CreateTableState,
    foreign_key_id: u64,
    column_index: usize,
) {
    let Some(foreign_key) = create
        .foreign_keys
        .iter_mut()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
    else {
        return;
    };
    if column_index > 0 && column_index < foreign_key.columns.len() {
        foreign_key.columns.swap(column_index - 1, column_index);
    }
}

fn create_table_move_foreign_key_column_down(
    create: &mut CreateTableState,
    foreign_key_id: u64,
    column_index: usize,
) {
    let Some(foreign_key) = create
        .foreign_keys
        .iter_mut()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
    else {
        return;
    };
    if column_index + 1 < foreign_key.columns.len() {
        foreign_key.columns.swap(column_index, column_index + 1);
    }
}

fn create_table_remove_foreign_key_column(
    create: &mut CreateTableState,
    foreign_key_id: u64,
    column_index: usize,
) {
    let Some(foreign_key) = create
        .foreign_keys
        .iter_mut()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
    else {
        return;
    };
    if column_index < foreign_key.columns.len() {
        foreign_key.columns.remove(column_index);
    }
}

fn create_table_set_foreign_key_column(
    create: &mut CreateTableState,
    foreign_key_id: u64,
    column_index: usize,
    value: String,
) {
    let Some(foreign_key) = create
        .foreign_keys
        .iter_mut()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
    else {
        return;
    };
    let Some(column) = foreign_key.columns.get_mut(column_index) else {
        return;
    };
    *column = value;
    create.autofill_empty_foreign_key_names();
}

fn create_table_start_foreign_key_reference_columns_load(
    create: &mut CreateTableState,
    foreign_key_id: u64,
) {
    let Some(foreign_key) = create
        .foreign_keys
        .iter_mut()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
    else {
        return;
    };
    if !foreign_key.referenced_table.trim().is_empty() {
        foreign_key.referenced_column_options = LoadState::Loading;
    }
}

fn create_table_finish_foreign_key_reference_columns_load(
    create: &mut CreateTableState,
    foreign_key_id: u64,
    result: std::result::Result<Vec<String>, UserFacingError>,
) {
    let Some(foreign_key) = create
        .foreign_keys
        .iter_mut()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
    else {
        return;
    };
    match result {
        Ok(columns) => foreign_key.referenced_column_options = LoadState::Loaded(columns),
        Err(error) => foreign_key.referenced_column_options = LoadState::Failed(error),
    }
}

fn create_table_add_foreign_key_referenced_column(
    create: &mut CreateTableState,
    foreign_key_id: u64,
) {
    let next_column = create
        .foreign_keys
        .iter()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
        .map(|foreign_key| create_table_next_referenced_column(foreign_key))
        .unwrap_or_default();
    let Some(foreign_key) = create
        .foreign_keys
        .iter_mut()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
    else {
        return;
    };
    foreign_key.referenced_columns.push(next_column);
    create.selected_foreign_key_id = Some(foreign_key_id);
}

fn create_table_move_foreign_key_referenced_column_up(
    create: &mut CreateTableState,
    foreign_key_id: u64,
    column_index: usize,
) {
    let Some(foreign_key) = create
        .foreign_keys
        .iter_mut()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
    else {
        return;
    };
    if column_index > 0 && column_index < foreign_key.referenced_columns.len() {
        foreign_key
            .referenced_columns
            .swap(column_index - 1, column_index);
    }
}

fn create_table_move_foreign_key_referenced_column_down(
    create: &mut CreateTableState,
    foreign_key_id: u64,
    column_index: usize,
) {
    let Some(foreign_key) = create
        .foreign_keys
        .iter_mut()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
    else {
        return;
    };
    if column_index + 1 < foreign_key.referenced_columns.len() {
        foreign_key
            .referenced_columns
            .swap(column_index, column_index + 1);
    }
}

fn create_table_remove_foreign_key_referenced_column(
    create: &mut CreateTableState,
    foreign_key_id: u64,
    column_index: usize,
) {
    let Some(foreign_key) = create
        .foreign_keys
        .iter_mut()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
    else {
        return;
    };
    if column_index < foreign_key.referenced_columns.len() {
        foreign_key.referenced_columns.remove(column_index);
    }
}

fn create_table_set_foreign_key_referenced_column(
    create: &mut CreateTableState,
    foreign_key_id: u64,
    column_index: usize,
    value: String,
) {
    let Some(foreign_key) = create
        .foreign_keys
        .iter_mut()
        .find(|foreign_key| foreign_key.id == foreign_key_id)
    else {
        return;
    };
    let Some(column) = foreign_key.referenced_columns.get_mut(column_index) else {
        return;
    };
    *column = value;
}

fn create_table_autofill_empty_foreign_key_names(create: &mut CreateTableState) {
    let table_name = create.table_name.trim();
    if table_name.is_empty() {
        return;
    }
    for foreign_key in &mut create.foreign_keys {
        if foreign_key.name.trim().is_empty() {
            foreign_key.name =
                create_table_auto_foreign_key_name(table_name, foreign_key.columns.iter());
        }
    }
}

fn create_table_foreign_key_validation_error(create: &CreateTableState) -> Option<&'static str> {
    let valid_columns = create_table_valid_column_names(create);
    let mut names = BTreeSet::new();
    for foreign_key in &create.foreign_keys {
        if foreign_key.name.trim().is_empty() {
            return Some("外键名称不能为空");
        }
        if !names.insert(foreign_key.name.trim().to_ascii_lowercase()) {
            return Some("外键名称不能重复");
        }
        if foreign_key.columns.is_empty() {
            return Some("外键至少需要选择一个字段");
        }
        let mut local_columns = BTreeSet::new();
        for column in &foreign_key.columns {
            let column_name = column.trim();
            if column_name.is_empty() {
                return Some("外键字段不能为空");
            }
            if !valid_columns.contains(&column_name.to_ascii_lowercase()) {
                return Some("外键引用了不存在的字段");
            }
            if !local_columns.insert(column_name.to_ascii_lowercase()) {
                return Some("外键字段不能重复");
            }
        }
        if foreign_key.referenced_table.trim().is_empty() {
            return Some("请选择目标表");
        }
        let referenced_columns = create_table_foreign_key_referenced_columns(foreign_key);
        if referenced_columns.is_empty() {
            return Some("目标字段不能为空");
        }
        if referenced_columns.len() != foreign_key.columns.len() {
            return Some("目标字段数量需要与外键字段一致");
        }
        let mut target_columns = BTreeSet::new();
        for column in &referenced_columns {
            if !target_columns.insert(column.to_ascii_lowercase()) {
                return Some("目标字段不能重复");
            }
        }
    }
    None
}

fn create_table_foreign_key_lines(
    create: &CreateTableState,
    dialect: CreateTableSqlDialect,
) -> Result<Vec<String>, String> {
    let valid_columns = create_table_valid_column_names(create);
    let mut names = BTreeSet::new();
    let mut lines = Vec::with_capacity(create.foreign_keys.len());
    for foreign_key in &create.foreign_keys {
        let name = foreign_key.name.trim();
        if name.is_empty() {
            return Err("Foreign key name is required.".to_string());
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(format!("Duplicate foreign key name: {name}."));
        }
        let mut columns = Vec::with_capacity(foreign_key.columns.len());
        let mut local_names = BTreeSet::new();
        for column in &foreign_key.columns {
            let column = column.trim();
            if column.is_empty() {
                return Err(format!("Foreign key {name} has an empty field."));
            }
            if !valid_columns.contains(&column.to_ascii_lowercase()) {
                return Err(format!("Foreign key {name} references unknown field: {column}."));
            }
            if !local_names.insert(column.to_ascii_lowercase()) {
                return Err(format!("Foreign key {name} has duplicate field: {column}."));
            }
            columns.push(match dialect {
                CreateTableSqlDialect::MySql => quote_mysql_identifier(column),
                CreateTableSqlDialect::Sqlite => quote_sqlite_identifier(column),
            });
        }
        if columns.is_empty() {
            return Err(format!("Foreign key {name} needs at least one field."));
        }
        let referenced_table = foreign_key.referenced_table.trim();
        if referenced_table.is_empty() {
            return Err(format!("Foreign key {name} needs a target table."));
        }
        let referenced_columns = create_table_foreign_key_referenced_columns(foreign_key);
        if referenced_columns.is_empty() {
            return Err(format!("Foreign key {name} needs target fields."));
        }
        if referenced_columns.len() != columns.len() {
            return Err(format!(
                "Foreign key {name} target field count must match local fields."
            ));
        }

        let mut line = match dialect {
            CreateTableSqlDialect::MySql => format!(
                "CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {}",
                quote_mysql_identifier(name),
                columns.join(", "),
                create_table_mysql_referenced_table(create, foreign_key)
            ),
            CreateTableSqlDialect::Sqlite => format!(
                "CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {}",
                quote_sqlite_identifier(name),
                columns.join(", "),
                quote_sqlite_identifier(referenced_table)
            ),
        };
        let quoted = referenced_columns
            .iter()
            .map(|column| match dialect {
                CreateTableSqlDialect::MySql => quote_mysql_identifier(column),
                CreateTableSqlDialect::Sqlite => quote_sqlite_identifier(column),
            })
            .collect::<Vec<_>>();
        line.push_str(" (");
        line.push_str(&quoted.join(", "));
        line.push(')');
        if !foreign_key.on_delete.trim().is_empty() {
            line.push_str(" ON DELETE ");
            line.push_str(create_table_normalized_foreign_key_action(&foreign_key.on_delete));
        }
        if !foreign_key.on_update.trim().is_empty() {
            line.push_str(" ON UPDATE ");
            line.push_str(create_table_normalized_foreign_key_action(&foreign_key.on_update));
        }
        lines.push(line);
    }
    Ok(lines)
}

fn create_table_mysql_referenced_table(
    create: &CreateTableState,
    foreign_key: &CreateTableForeignKey,
) -> String {
    let referenced_database = foreign_key.referenced_database.trim();
    let current_database = create.database.as_deref().unwrap_or("").trim();
    if !referenced_database.is_empty() && referenced_database != current_database {
        format!(
            "{}.{}",
            quote_mysql_identifier(referenced_database),
            quote_mysql_identifier(foreign_key.referenced_table.trim())
        )
    } else {
        quote_mysql_identifier(foreign_key.referenced_table.trim())
    }
}

fn create_table_valid_column_names(create: &CreateTableState) -> BTreeSet<String> {
    create
        .columns
        .iter()
        .map(|column| column.name.trim())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_ascii_lowercase())
        .collect()
}

fn create_table_foreign_key_referenced_columns(
    foreign_key: &CreateTableForeignKey,
) -> Vec<String> {
    foreign_key
        .referenced_columns
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn create_table_next_referenced_column(foreign_key: &CreateTableForeignKey) -> String {
    let used = foreign_key
        .referenced_columns
        .iter()
        .map(|column| column.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let options = match &foreign_key.referenced_column_options {
        LoadState::Loaded(options) => options.as_slice(),
        _ => &[],
    };
    options
        .iter()
        .map(|name| name.trim())
        .find(|name| !name.is_empty() && !used.contains(&name.to_ascii_lowercase()))
        .map(str::to_string)
        .unwrap_or_default()
}

fn create_table_normalized_foreign_key_action(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "CASCADE" => "CASCADE",
        "NO ACTION" => "NO ACTION",
        "RESTRICT" => "RESTRICT",
        "SET NULL" => "SET NULL",
        _ => "",
    }
}

fn create_table_auto_foreign_key_name<'a>(
    table_name: &str,
    columns: impl Iterator<Item = &'a String>,
) -> String {
    let columns = columns
        .map(|column| column.trim())
        .filter(|column| !column.is_empty())
        .collect::<Vec<_>>();
    if columns.is_empty() {
        return String::new();
    }
    let mut name = format!("fk_{table_name}_{}", columns.join("_"));
    if name.len() > 64 {
        name.truncate(64);
    }
    name
}

impl CreateTableForeignKey {
    fn new(id: u64, database: Option<String>) -> Self {
        Self {
            id,
            name: String::new(),
            columns: Vec::new(),
            referenced_database: database.unwrap_or_default(),
            referenced_table: String::new(),
            referenced_columns: Vec::new(),
            referenced_column_options: LoadState::NotLoaded,
            on_delete: String::new(),
            on_update: String::new(),
        }
    }
}
