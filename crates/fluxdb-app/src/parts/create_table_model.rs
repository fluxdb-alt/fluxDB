#[derive(Clone, Debug, PartialEq)]
pub struct CreateTableState {
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    /// schema 作用域（PG：表所属 schema，空串表示用连接默认 search_path / public）。
    pub schema: String,
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

