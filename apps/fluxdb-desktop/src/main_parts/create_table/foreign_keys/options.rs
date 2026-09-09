fn create_table_reference_database_options(
    create: &CreateTableState,
    state: &AppState,
) -> Vec<String> {
    let mut options = BTreeSet::new();
    if let Some(database) = create.database.as_deref().filter(|database| !database.is_empty()) {
        options.insert(database.to_string());
    }
    if let Some(connection) = state
        .connections
        .iter()
        .find(|connection| connection.config.id == create.connection_id)
    {
        for object in &connection.objects {
            if matches!(object.path.kind, ObjectKind::Database | ObjectKind::Schema) {
                let name = object
                    .path
                    .database
                    .as_deref()
                    .unwrap_or(object.path.name.as_str());
                if !name.trim().is_empty() {
                    options.insert(name.to_string());
                }
            }
        }
    }
    options.into_iter().collect()
}

fn create_table_reference_table_options(
    create: &CreateTableState,
    state: &AppState,
    foreign_key: &CreateTableForeignKey,
) -> Vec<String> {
    let database = create_table_foreign_key_effective_database(create, foreign_key);
    let mut options = BTreeSet::new();
    if let Some(connection) = state
        .connections
        .iter()
        .find(|connection| connection.config.id == create.connection_id)
    {
        for object in &connection.objects {
            if object.path.kind == ObjectKind::Table
                && object.path.database.as_deref().unwrap_or("main") == database
            {
                options.insert(object.path.name.clone());
            }
        }
    }
    if !foreign_key.referenced_table.trim().is_empty() {
        options.insert(foreign_key.referenced_table.trim().to_string());
    }
    options.into_iter().collect()
}

fn create_table_reference_database_path(
    state: &AppState,
    tab_id: TabId,
    database: &str,
) -> Option<ObjectPath> {
    let create = state.tabs.iter().find_map(|tab| match &tab.kind {
        TabKind::CreateTable(create) if tab.id == tab_id => Some(create),
        _ => None,
    })?;
    state
        .connections
        .iter()
        .find(|connection| connection.config.id == create.connection_id)
        .and_then(|connection| {
            connection.objects.iter().find_map(|object| {
                let name = object
                    .path
                    .database
                    .as_deref()
                    .unwrap_or(object.path.name.as_str());
                (matches!(object.path.kind, ObjectKind::Database | ObjectKind::Schema)
                    && name == database)
                    .then_some(object.path.clone())
            })
        })
        .or_else(|| {
            Some(ObjectPath {
                connection_id: create.connection_id,
                database: Some(database.to_string()),
                schema: None,
                name: database.to_string(),
                kind: ObjectKind::Database,
            })
        })
}

fn create_table_foreign_key_effective_database(
    create: &CreateTableState,
    foreign_key: &CreateTableForeignKey,
) -> String {
    let referenced_database = foreign_key.referenced_database.trim();
    if !referenced_database.is_empty() {
        return referenced_database.to_string();
    }
    create
        .database
        .as_deref()
        .unwrap_or(if create.database_kind == DatabaseKind::Sqlite {
            "main"
        } else {
            ""
        })
        .to_string()
}

fn create_table_foreign_key_action_options() -> Vec<String> {
    ["", "CASCADE", "NO ACTION", "RESTRICT", "SET NULL"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn create_table_foreign_key_fields_summary(foreign_key: &CreateTableForeignKey) -> String {
    foreign_key
        .columns
        .iter()
        .filter(|column| !column.trim().is_empty())
        .map(|column| create_table_quote_identifier(column.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn create_table_foreign_key_referenced_fields_summary(
    foreign_key: &CreateTableForeignKey,
) -> String {
    foreign_key
        .referenced_columns
        .iter()
        .filter(|column| !column.trim().is_empty())
        .map(|column| create_table_quote_identifier(column.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn create_table_foreign_key_referenced_field_options(
    foreign_key: &CreateTableForeignKey,
) -> Vec<String> {
    let mut options = BTreeSet::new();
    if let LoadState::Loaded(columns) = &foreign_key.referenced_column_options {
        for column in columns {
            if !column.trim().is_empty() {
                options.insert(column.trim().to_string());
            }
        }
    }
    for column in &foreign_key.referenced_columns {
        if !column.trim().is_empty() {
            options.insert(column.trim().to_string());
        }
    }
    options.into_iter().collect()
}
