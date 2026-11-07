pub trait CreateTableProvider {
    fn type_options(&self) -> &'static [&'static str];
    fn default_column_type(&self) -> &'static str;
    fn default_id_type(&self) -> &'static str;
    fn default_length(&self, data_type: &str) -> Option<&'static str>;
    /// 把元数据返回的完整类型文本规范化为设计表解析用的基础形式。
    fn normalize_metadata_type(&self, raw_type: &str) -> String {
        raw_type.to_string()
    }
    fn type_capabilities(&self, data_type: &str) -> CreateTableTypeCapabilities;
    fn validation_error(&self, create: &CreateTableState) -> Option<&'static str>;
    fn sql_preview(&self, create: &CreateTableState) -> Result<String, String>;
    fn design_sql_preview(&self, create: &CreateTableState) -> Result<String, String>;
    fn design_statements(&self, create: &CreateTableState) -> Result<Vec<String>, String>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CreateTableTypeCapabilities {
    pub length: bool,
    pub bit_length: bool,
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

/// 返回该方言用于“新建表”的基础类型。特殊/扩展类型放在末尾，由界面单独分组。
pub fn create_table_common_type_options(kind: DatabaseKind) -> Vec<String> {
    let options = create_table_provider(kind).type_options();
    options
        .iter()
        .filter(|option| !create_table_type_is_special(kind, option))
        .map(|option| option.to_string())
        .collect()
}

pub fn create_table_special_type_options(kind: DatabaseKind) -> Vec<String> {
    create_table_provider(kind)
        .type_options()
        .iter()
        .filter(|option| create_table_type_is_special(kind, option))
        .map(|option| option.to_string())
        .collect()
}

fn create_table_type_is_special(kind: DatabaseKind, data_type: &str) -> bool {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => matches!(
            data_type,
            "enum"
                | "set"
                | "geometry"
                | "point"
                | "linestring"
                | "polygon"
                | "multipoint"
                | "multilinestring"
                | "multipolygon"
                | "geometrycollection"
                | "vector"
        ),
        DatabaseKind::Postgres | DatabaseKind::Kingbase => matches!(
            data_type,
            "oid"
                | "regclass"
                | "regproc"
                | "tsvector"
                | "tsquery"
                | "point"
                | "line"
                | "lseg"
                | "box"
                | "path"
                | "polygon"
                | "circle"
                | "int4range"
                | "int8range"
                | "numrange"
                | "tsrange"
                | "tstzrange"
                | "daterange"
                | "citext"
                | "hstore"
                | "ltree"
        ),
        DatabaseKind::Sqlite => matches!(data_type, "any" | "enum" | "set"),
        _ => false,
    }
}

pub fn create_table_provider(kind: DatabaseKind) -> &'static dyn CreateTableProvider {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
            &MYSQL_CREATE_TABLE_PROVIDER
        }
        DatabaseKind::Sqlite => &SQLITE_CREATE_TABLE_PROVIDER,
        DatabaseKind::Postgres | DatabaseKind::Kingbase => &POSTGRES_CREATE_TABLE_PROVIDER,
        DatabaseKind::Dameng => &DAMENG_CREATE_TABLE_PROVIDER,
        DatabaseKind::SqlServer => &SQLSERVER_CREATE_TABLE_PROVIDER,
        DatabaseKind::Oracle => &ORACLE_CREATE_TABLE_PROVIDER,
        DatabaseKind::MongoDb | DatabaseKind::Redis => {
            &UNSUPPORTED_CREATE_TABLE_PROVIDER
        }
    }
}

static POSTGRES_CREATE_TABLE_PROVIDER: PostgresCreateTableProvider = PostgresCreateTableProvider;
static DAMENG_CREATE_TABLE_PROVIDER: DamengCreateTableProvider = DamengCreateTableProvider;
static ORACLE_CREATE_TABLE_PROVIDER: OracleCreateTableProvider = OracleCreateTableProvider;
static SQLSERVER_CREATE_TABLE_PROVIDER: SqlServerCreateTableProvider =
    SqlServerCreateTableProvider;

struct SqlServerCreateTableProvider;

impl CreateTableProvider for SqlServerCreateTableProvider {
    fn type_options(&self) -> &'static [&'static str] {
        SQLSERVER_CREATE_TABLE_TYPE_OPTIONS
    }

    fn default_column_type(&self) -> &'static str {
        "nvarchar"
    }

    fn default_id_type(&self) -> &'static str {
        "int"
    }

    fn default_length(&self, data_type: &str) -> Option<&'static str> {
        match create_table_base_type(data_type).as_str() {
            "char" | "varchar" | "nchar" | "nvarchar" | "binary" | "varbinary" => Some("255"),
            "decimal" | "numeric" => Some("18"),
            _ => None,
        }
    }

    fn type_capabilities(&self, data_type: &str) -> CreateTableTypeCapabilities {
        let base = create_table_base_type(data_type);
        CreateTableTypeCapabilities {
            length: matches!(
                base.as_str(),
                "char"
                    | "varchar"
                    | "nchar"
                    | "nvarchar"
                    | "binary"
                    | "varbinary"
                    | "decimal"
                    | "numeric"
            ),
            scale: matches!(
                base.as_str(),
                "decimal" | "numeric" | "time" | "datetime2" | "datetimeoffset"
            ),
            number_options: matches!(base.as_str(), "decimal" | "numeric"),
            auto_increment: sqlserver_is_identity_type(&base),
            ..Default::default()
        }
    }

    fn validation_error(&self, create: &CreateTableState) -> Option<&'static str> {
        create_table_sqlserver_validation_error(create)
    }

    fn sql_preview(&self, create: &CreateTableState) -> Result<String, String> {
        create_table_sqlserver_sql_preview(create)
    }

    fn design_sql_preview(&self, create: &CreateTableState) -> Result<String, String> {
        create_table_design_sql_preview_for_provider(self, create)
    }

    fn design_statements(&self, create: &CreateTableState) -> Result<Vec<String>, String> {
        create_table_sqlserver_design_statements(create)
    }
}

struct DamengCreateTableProvider;

impl CreateTableProvider for DamengCreateTableProvider {
    fn type_options(&self) -> &'static [&'static str] {
        DAMENG_CREATE_TABLE_TYPE_OPTIONS
    }

    fn default_column_type(&self) -> &'static str {
        "varchar"
    }

    fn default_id_type(&self) -> &'static str {
        "int"
    }

    fn default_length(&self, data_type: &str) -> Option<&'static str> {
        match create_table_base_type(data_type).as_str() {
            "char" | "varchar" | "varchar2" | "nchar" | "nvarchar" | "nvarchar2" => Some("255"),
            "number" | "numeric" | "decimal" => Some("10"),
            _ => None,
        }
    }

    fn type_capabilities(&self, data_type: &str) -> CreateTableTypeCapabilities {
        let base = create_table_base_type(data_type);
        CreateTableTypeCapabilities {
            length: dameng_supports_length(&base),
            scale: dameng_supports_scale(&base),
            number_options: dameng_supports_scale(&base),
            auto_increment: dameng_is_integer_type(&base),
            ..Default::default()
        }
    }

    fn validation_error(&self, create: &CreateTableState) -> Option<&'static str> {
        create_table_dameng_validation_error(create)
    }

    fn sql_preview(&self, create: &CreateTableState) -> Result<String, String> {
        create_table_dameng_sql_preview(create)
    }

    fn design_sql_preview(&self, create: &CreateTableState) -> Result<String, String> {
        create_table_design_sql_preview_for_provider(self, create)
    }

    fn design_statements(&self, create: &CreateTableState) -> Result<Vec<String>, String> {
        create_table_dameng_design_statements(create)
    }
}

/// Oracle 建表 provider。
///
/// 与达梦的关键差异：自增用 identity 子句而不是 `IDENTITY(1,1)`；改列类型必须
/// 走 `ALTER TABLE … MODIFY`；没有 `UNSIGNED`/`ZEROFILL`/前缀索引。
struct OracleCreateTableProvider;

impl CreateTableProvider for OracleCreateTableProvider {
    fn type_options(&self) -> &'static [&'static str] {
        ORACLE_CREATE_TABLE_TYPE_OPTIONS
    }

    fn default_column_type(&self) -> &'static str {
        "varchar2"
    }

    fn default_id_type(&self) -> &'static str {
        "number"
    }

    fn default_length(&self, data_type: &str) -> Option<&'static str> {
        match create_table_base_type(data_type).as_str() {
            "char" | "varchar" | "varchar2" | "nchar" | "nvarchar2" | "raw" => Some("255"),
            "number" | "numeric" | "decimal" => Some("10"),
            _ => None,
        }
    }

    fn type_capabilities(&self, data_type: &str) -> CreateTableTypeCapabilities {
        let base = create_table_base_type(data_type);
        CreateTableTypeCapabilities {
            length: oracle_supports_length(&base),
            scale: oracle_supports_scale(&base),
            number_options: oracle_supports_scale(&base),
            auto_increment: oracle_is_numeric_type(&base),
            ..Default::default()
        }
    }

    fn validation_error(&self, create: &CreateTableState) -> Option<&'static str> {
        create_table_oracle_validation_error(create)
    }

    fn sql_preview(&self, create: &CreateTableState) -> Result<String, String> {
        create_table_oracle_sql_preview(create)
    }

    fn design_sql_preview(&self, create: &CreateTableState) -> Result<String, String> {
        create_table_design_sql_preview_for_provider(self, create)
    }

    fn design_statements(&self, create: &CreateTableState) -> Result<Vec<String>, String> {
        create_table_oracle_design_statements(create)
    }
}

/// PostgreSQL 建表 provider（T16，设计 §9.2）。
///
/// 与 MySQL 的差异集中在这里体现：没有 engine/charset/unsigned/zerofill/ON UPDATE/
/// 索引前缀长度/认证插件；identity 用 `GENERATED BY DEFAULT AS IDENTITY`；
/// 注释是独立的 `COMMENT ON`；触发器引用已存在函数而非行内 BEGIN…END。
struct PostgresCreateTableProvider;

impl CreateTableProvider for PostgresCreateTableProvider {
    fn type_options(&self) -> &'static [&'static str] {
        POSTGRES_CREATE_TABLE_TYPE_OPTIONS
    }

    fn default_column_type(&self) -> &'static str {
        "text"
    }

    fn default_id_type(&self) -> &'static str {
        "integer"
    }

    fn default_length(&self, data_type: &str) -> Option<&'static str> {
        // PG 的 varchar/numeric 不写长度即为无限制；只有 char 习惯给长度。
        match data_type.trim().to_ascii_lowercase().as_str() {
            "char" | "bpchar" => Some("1"),
            _ => None,
        }
    }

    fn type_capabilities(&self, data_type: &str) -> CreateTableTypeCapabilities {
        // 完整定义由元数据层原样保留，不能按括号前的标量类型开放参数或 identity。
        if data_type.contains(['(', '[', '"', '.']) {
            return CreateTableTypeCapabilities::default();
        }
        let base = create_table_base_type(data_type);
        let bit_length = matches!(base.as_str(), "bit" | "varbit");
        // numeric 精度和 timestamp/time 的小数秒精度都复用 length 输入。
        let length = matches!(
            base.as_str(),
            "varchar"
                | "char"
                | "bpchar"
                | "numeric"
                | "decimal"
                | "timestamp"
                | "timestamptz"
                | "time"
                | "timetz"
        );
        let scale = matches!(base.as_str(), "numeric" | "decimal");
        CreateTableTypeCapabilities {
            length,
            bit_length,
            scale,
            text_options: false,
            key_length: false,
            number_options: scale,
            unsigned: false,
            zerofill: false,
            // PG 用 identity 而非 AUTO_INCREMENT；能力位复用为「可设 identity」。
            auto_increment: postgres_is_integer_type(data_type),
            auto_update_time: false,
            binary_attribute: false,
        }
    }

    fn validation_error(&self, create: &CreateTableState) -> Option<&'static str> {
        // PG 专属约束先判：identity 只能挂整数列、触发器必须引用已存在函数——
        // 这两条比通用校验更精确，先给出 PG 语义的提示再落到通用结构校验。
        if create
            .columns
            .iter()
            .any(|column| column.auto_increment && !postgres_is_integer_type(&column.data_type))
        {
            return Some("PostgreSQL identity 只能用于整数类型列");
        }
        if create
            .triggers
            .iter()
            .any(|trigger| postgres_trigger_error(trigger).is_some())
        {
            return Some("PostgreSQL 触发器需要引用已存在的函数，不支持行内 BEGIN…END 触发器体");
        }
        // 复用通用校验（名称/主键/索引/FK/check 结构）。
        create_table_mysql_validation_error(create)
    }

    fn sql_preview(&self, create: &CreateTableState) -> Result<String, String> {
        create_table_postgres_sql_preview(create)
    }

    fn design_sql_preview(&self, create: &CreateTableState) -> Result<String, String> {
        create_table_design_sql_preview_for_provider(self, create)
    }

    fn design_statements(&self, create: &CreateTableState) -> Result<Vec<String>, String> {
        create_table_postgres_design_statements(create)
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
    "year",
    "bit",
    "enum",
    "set",
    "geometry",
    "point",
    "linestring",
    "polygon",
    "multipoint",
    "multilinestring",
    "multipolygon",
    "geometrycollection",
    "vector",
];

const SQLITE_CREATE_TABLE_TYPE_OPTIONS: &[&str] = &[
    "integer",
    "real",
    "numeric",
    "text",
    "blob",
    "boolean",
    "date",
    "datetime",
    "json",
    "any",
    "char",
    "varchar",
    "nchar",
    "nvarchar",
    "clob",
    "double",
    "double precision",
    "float",
    "decimal",
    "int",
    "tinyint",
    "smallint",
    "mediumint",
    "bigint",
    "unsigned big int",
    "enum",
    "set",
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

    fn normalize_metadata_type(&self, raw_type: &str) -> String {
        // column_type 会把 unsigned/zerofill 拼在类型后；设计表用独立开关承载这两个属性。
        create_table_strip_mysql_type_modifiers(raw_type)
    }

    fn type_capabilities(&self, data_type: &str) -> CreateTableTypeCapabilities {
        let base = create_table_base_type(data_type);
        let bit_length = base == "bit";
        CreateTableTypeCapabilities {
            length: create_table_mysql_supports_length(data_type),
            bit_length,
            scale: create_table_mysql_supports_scale(data_type),
            text_options: create_table_mysql_is_text_type(data_type),
            key_length: create_table_mysql_supports_key_length(data_type),
            number_options: create_table_mysql_is_number_type(data_type),
            unsigned: create_table_mysql_is_number_type(data_type)
                && create_table_base_type(data_type) != "year",
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
            length: create_table_sqlite_supports_length(&base),
            scale: create_table_sqlite_supports_scale(&base),
            number_options: create_table_sqlite_supports_scale(&base),
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
