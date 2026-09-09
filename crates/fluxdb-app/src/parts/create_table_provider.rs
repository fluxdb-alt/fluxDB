pub trait CreateTableProvider {
    fn type_options(&self) -> &'static [&'static str];
    fn default_column_type(&self) -> &'static str;
    fn default_id_type(&self) -> &'static str;
    fn default_length(&self, data_type: &str) -> Option<&'static str>;
    fn type_capabilities(&self, data_type: &str) -> CreateTableTypeCapabilities;
    fn validation_error(&self, create: &CreateTableState) -> Option<&'static str>;
    fn sql_preview(&self, create: &CreateTableState) -> Result<String, String>;
    fn design_sql_preview(&self, create: &CreateTableState) -> Result<String, String>;
    fn design_statements(&self, create: &CreateTableState) -> Result<Vec<String>, String>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CreateTableTypeCapabilities {
    pub length: bool,
    pub scale: bool,
    pub text_options: bool,
    pub key_length: bool,
    pub number_options: bool,
    pub unsigned: bool,
    pub zerofill: bool,
    pub auto_increment: bool,
    pub auto_update_time: bool,
    pub binary_attribute: bool,
}

pub fn create_table_provider(kind: DatabaseKind) -> &'static dyn CreateTableProvider {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => &MYSQL_CREATE_TABLE_PROVIDER,
        DatabaseKind::Sqlite => &SQLITE_CREATE_TABLE_PROVIDER,
        DatabaseKind::MongoDb | DatabaseKind::Redis => &UNSUPPORTED_CREATE_TABLE_PROVIDER,
    }
}

static MYSQL_CREATE_TABLE_PROVIDER: MySqlCreateTableProvider = MySqlCreateTableProvider;
static SQLITE_CREATE_TABLE_PROVIDER: SqliteCreateTableProvider = SqliteCreateTableProvider;
static UNSUPPORTED_CREATE_TABLE_PROVIDER: UnsupportedCreateTableProvider =
    UnsupportedCreateTableProvider;

const MYSQL_CREATE_TABLE_TYPE_OPTIONS: &[&str] = &[
    "tinyint",
    "smallint",
    "mediumint",
    "int",
    "integer",
    "bigint",
    "float",
    "double",
    "decimal",
    "varchar",
    "char",
    "binary",
    "varbinary",
    "tinytext",
    "text",
    "mediumtext",
    "longtext",
    "tinyblob",
    "blob",
    "mediumblob",
    "longblob",
    "date",
    "datetime",
    "timestamp",
    "time",
    "json",
    "boolean",
];

const SQLITE_CREATE_TABLE_TYPE_OPTIONS: &[&str] = &[
    "integer", "real", "numeric", "text", "blob", "boolean", "date", "datetime", "json",
];

struct MySqlCreateTableProvider;

impl CreateTableProvider for MySqlCreateTableProvider {
    fn type_options(&self) -> &'static [&'static str] {
        MYSQL_CREATE_TABLE_TYPE_OPTIONS
    }

    fn default_column_type(&self) -> &'static str {
        "varchar"
    }

    fn default_id_type(&self) -> &'static str {
        "int"
    }

    fn default_length(&self, data_type: &str) -> Option<&'static str> {
        create_table_mysql_default_length(data_type)
    }

    fn type_capabilities(&self, data_type: &str) -> CreateTableTypeCapabilities {
        CreateTableTypeCapabilities {
            length: create_table_mysql_supports_length(data_type),
            scale: create_table_mysql_supports_scale(data_type),
            text_options: create_table_mysql_is_text_type(data_type),
            key_length: create_table_mysql_supports_key_length(data_type),
            number_options: create_table_mysql_is_number_type(data_type),
            unsigned: create_table_mysql_is_number_type(data_type),
            zerofill: create_table_mysql_is_number_type(data_type),
            auto_increment: create_table_mysql_is_number_type(data_type),
            auto_update_time: create_table_mysql_is_auto_update_time_type(data_type),
            binary_attribute: create_table_mysql_is_text_type(data_type),
        }
    }

    fn validation_error(&self, create: &CreateTableState) -> Option<&'static str> {
        create_table_mysql_validation_error(create)
    }

    fn sql_preview(&self, create: &CreateTableState) -> Result<String, String> {
        create_table_mysql_sql_preview(create)
    }

    fn design_sql_preview(&self, create: &CreateTableState) -> Result<String, String> {
        create_table_design_sql_preview_for_provider(self, create)
    }

    fn design_statements(&self, create: &CreateTableState) -> Result<Vec<String>, String> {
        create_table_mysql_design_statements(create)
    }
}

struct SqliteCreateTableProvider;

impl CreateTableProvider for SqliteCreateTableProvider {
    fn type_options(&self) -> &'static [&'static str] {
        SQLITE_CREATE_TABLE_TYPE_OPTIONS
    }

    fn default_column_type(&self) -> &'static str {
        "text"
    }

    fn default_id_type(&self) -> &'static str {
        "integer"
    }

    fn default_length(&self, _: &str) -> Option<&'static str> {
        None
    }

    fn type_capabilities(&self, data_type: &str) -> CreateTableTypeCapabilities {
        let base = create_table_base_type(data_type);
        let integer = base == "integer" || base == "int";
        CreateTableTypeCapabilities {
            auto_increment: integer,
            auto_update_time: false,
            ..Default::default()
        }
    }

    fn validation_error(&self, create: &CreateTableState) -> Option<&'static str> {
        if let Some(message) = create_table_mysql_validation_error(create) {
            return Some(message);
        }
        if create
            .columns
            .iter()
            .any(|column| column.auto_increment && !column.primary_key)
        {
            return Some("SQLite 自增字段必须是整数主键");
        }
        let auto_increment_columns = create
            .columns
            .iter()
            .filter(|column| column.auto_increment)
            .collect::<Vec<_>>();
        if let Some(column) = auto_increment_columns.first()
            && (auto_increment_columns.len() > 1
                || create.columns.iter().filter(|column| column.primary_key).count() > 1
                || !matches!(
                    create_table_base_type(&column.data_type).as_str(),
                    "integer" | "int"
                ))
        {
            return Some("SQLite 自增字段只能是单列整数主键");
        }
        None
    }

    fn sql_preview(&self, create: &CreateTableState) -> Result<String, String> {
        create_table_sqlite_sql_preview(create)
    }

    fn design_sql_preview(&self, create: &CreateTableState) -> Result<String, String> {
        create_table_design_sql_preview_for_provider(self, create)
    }

    fn design_statements(&self, create: &CreateTableState) -> Result<Vec<String>, String> {
        create_table_sqlite_design_statements(create)
    }
}

struct UnsupportedCreateTableProvider;

impl CreateTableProvider for UnsupportedCreateTableProvider {
    fn type_options(&self) -> &'static [&'static str] {
        &[]
    }

    fn default_column_type(&self) -> &'static str {
        ""
    }

    fn default_id_type(&self) -> &'static str {
        ""
    }

    fn default_length(&self, _: &str) -> Option<&'static str> {
        None
    }

    fn type_capabilities(&self, _: &str) -> CreateTableTypeCapabilities {
        CreateTableTypeCapabilities::default()
    }

    fn validation_error(&self, _: &CreateTableState) -> Option<&'static str> {
        Some("当前连接类型暂不支持创建表")
    }

    fn sql_preview(&self, _: &CreateTableState) -> Result<String, String> {
        Err("当前连接类型暂不支持创建表".to_string())
    }

    fn design_sql_preview(&self, _: &CreateTableState) -> Result<String, String> {
        Err("当前连接类型暂不支持设计表".to_string())
    }

    fn design_statements(&self, _: &CreateTableState) -> Result<Vec<String>, String> {
        Err("当前连接类型暂不支持设计表".to_string())
    }
}
