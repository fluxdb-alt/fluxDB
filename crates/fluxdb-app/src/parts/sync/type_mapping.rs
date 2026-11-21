//! 跨数据库类型的字段类型映射。
//!
//! 先把源方言类型解析成与方言无关的 [`NeutralType`]，再按目标方言渲染。这样新增
//! 一种数据库只需要补「解析」和「渲染」两面，不必维护 N×N 的映射矩阵。
//!
//! 原则：
//! - 映射结果必须能在目标库直接执行，长度/精度按目标上限夹取，夹取时给出警告；
//! - 无法表达的类型退回目标库的大文本类型并警告，绝不静默丢列；
//! - 默认值表达式只在能确定等价时翻译，跨方言函数一律丢弃并警告，避免写坏数据。

use fluxdb_core::DatabaseKind;

/// 与方言无关的类型描述。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NeutralType {
    Bool,
    Integer {
        bits: u8,
        unsigned: bool,
    },
    Decimal {
        precision: Option<u32>,
        scale: u32,
    },
    Float {
        bits: u8,
    },
    Text {
        len: Option<u32>,
        long: bool,
    },
    Binary {
        len: Option<u32>,
        long: bool,
    },
    Json,
    Date,
    Time {
        precision: Option<u32>,
        with_tz: bool,
    },
    Timestamp {
        precision: Option<u32>,
        with_tz: bool,
    },
    Uuid,
    Enum {
        values: Vec<String>,
    },
    /// 源类型无法识别：目标端用大文本兜底并提示。
    Unknown,
}

/// 类型映射结果：目标类型文本 + 需要展示给用户的告警。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MappedType {
    pub text: String,
    pub warnings: Vec<String>,
}

/// 默认值映射结果。`expr` 为 `None` 表示该默认值不能安全翻译，应丢弃。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MappedDefault {
    pub expr: Option<String>,
    pub warning: Option<String>,
}

/// 解析出的源类型语法片段。
struct ParsedType {
    base: String,
    args: String,
    unsigned: bool,
    is_array: bool,
}

fn parse_type(type_text: &str) -> ParsedType {
    let mut text = type_text.trim().to_ascii_lowercase();
    let is_array = text.ends_with("[]");
    if is_array {
        text.truncate(text.len() - 2);
        text = text.trim().to_string();
    }
    let unsigned = text.contains("unsigned");
    text = text
        .replace("unsigned", " ")
        .replace("zerofill", " ")
        .trim()
        .to_string();
    let (base, args) = match text.find('(') {
        Some(start) => {
            let end = text.rfind(')').unwrap_or(text.len());
            (
                text[..start].trim().to_string(),
                text[start + 1..end].trim().to_string(),
            )
        }
        None => (text, String::new()),
    };
    // `double precision` / `character varying` 这类多词基础类型保留空格。
    ParsedType {
        base: base.split_whitespace().collect::<Vec<_>>().join(" "),
        args,
        unsigned,
        is_array,
    }
}

fn numeric_args(args: &str) -> Vec<u32> {
    args.split(',')
        .filter_map(|part| part.trim().parse::<u32>().ok())
        .collect()
}

fn quoted_values(args: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    for ch in args.chars() {
        match ch {
            '\'' => {
                if in_quote {
                    values.push(current.clone());
                    current.clear();
                }
                in_quote = !in_quote;
            }
            _ if in_quote => current.push(ch),
            _ => {}
        }
    }
    values
}

fn text_len(len: Option<u32>) -> NeutralType {
    NeutralType::Text { len, long: false }
}

fn long_text() -> NeutralType {
    NeutralType::Text {
        len: None,
        long: true,
    }
}

fn binary_len(len: Option<u32>) -> NeutralType {
    NeutralType::Binary { len, long: false }
}

fn long_binary() -> NeutralType {
    NeutralType::Binary {
        len: None,
        long: true,
    }
}

/// 把源方言类型文本解析成中性类型。
pub(crate) fn neutral_type(source: DatabaseKind, type_text: &str) -> NeutralType {
    let parsed = parse_type(type_text);
    if parsed.is_array {
        return NeutralType::Unknown;
    }
    let args = numeric_args(&parsed.args);
    match source {
        kind if kind.is_mysql_family() => mysql_neutral(&parsed, &args),
        kind if kind.is_postgres_family() => postgres_neutral(&parsed, &args),
        DatabaseKind::Sqlite => sqlite_neutral(&parsed, &args),
        DatabaseKind::SqlServer => sqlserver_neutral(&parsed, &args),
        DatabaseKind::Oracle => oracle_neutral(&parsed, &args),
        DatabaseKind::Dameng => dameng_neutral(&parsed, &args),
        _ => NeutralType::Unknown,
    }
}

fn mysql_neutral(parsed: &ParsedType, args: &[u32]) -> NeutralType {
    match parsed.base.as_str() {
        "bool" | "boolean" => NeutralType::Bool,
        "bit" => match args.first() {
            Some(1) | None => NeutralType::Bool,
            _ => long_binary(),
        },
        "tinyint" => match args.first() {
            Some(1) => NeutralType::Bool,
            _ => NeutralType::Integer {
                bits: 8,
                unsigned: parsed.unsigned,
            },
        },
        "smallint" => NeutralType::Integer {
            bits: 16,
            unsigned: parsed.unsigned,
        },
        "mediumint" => NeutralType::Integer {
            bits: 24,
            unsigned: parsed.unsigned,
        },
        "int" | "integer" => NeutralType::Integer {
            bits: 32,
            unsigned: parsed.unsigned,
        },
        "bigint" => NeutralType::Integer {
            bits: 64,
            unsigned: parsed.unsigned,
        },
        "year" => NeutralType::Integer {
            bits: 16,
            unsigned: false,
        },
        "decimal" | "numeric" | "dec" | "fixed" => NeutralType::Decimal {
            precision: args.first().copied(),
            scale: args.get(1).copied().unwrap_or(0),
        },
        "float" => NeutralType::Float {
            bits: if args.first().is_some_and(|precision| *precision > 24) {
                64
            } else {
                32
            },
        },
        "double" | "real" => NeutralType::Float { bits: 64 },
        "char" | "varchar" | "nchar" | "nvarchar" => text_len(args.first().copied()),
        "tinytext" | "text" | "mediumtext" | "longtext" => long_text(),
        "binary" | "varbinary" => binary_len(args.first().copied()),
        "tinyblob" | "blob" | "mediumblob" | "longblob" => long_binary(),
        "json" => NeutralType::Json,
        "date" => NeutralType::Date,
        "time" => NeutralType::Time {
            precision: args.first().copied(),
            with_tz: false,
        },
        "datetime" | "timestamp" => NeutralType::Timestamp {
            precision: args.first().copied(),
            with_tz: false,
        },
        "enum" => NeutralType::Enum {
            values: quoted_values(&parsed.args),
        },
        "set" => long_text(),
        _ => NeutralType::Unknown,
    }
}

fn postgres_neutral(parsed: &ParsedType, args: &[u32]) -> NeutralType {
    match parsed.base.as_str() {
        "bool" | "boolean" => NeutralType::Bool,
        "int2" | "smallint" => NeutralType::Integer {
            bits: 16,
            unsigned: false,
        },
        "int4" | "int" | "integer" => NeutralType::Integer {
            bits: 32,
            unsigned: false,
        },
        "int8" | "bigint" => NeutralType::Integer {
            bits: 64,
            unsigned: false,
        },
        "serial" | "serial4" => NeutralType::Integer {
            bits: 32,
            unsigned: false,
        },
        "bigserial" | "serial8" => NeutralType::Integer {
            bits: 64,
            unsigned: false,
        },
        "smallserial" | "serial2" => NeutralType::Integer {
            bits: 16,
            unsigned: false,
        },
        "numeric" | "decimal" => NeutralType::Decimal {
            precision: args.first().copied(),
            scale: args.get(1).copied().unwrap_or(0),
        },
        "money" => NeutralType::Decimal {
            precision: Some(19),
            scale: 2,
        },
        "real" | "float4" => NeutralType::Float { bits: 32 },
        "double precision" | "float8" | "float" => NeutralType::Float { bits: 64 },
        "varchar" | "character varying" => text_len(args.first().copied()),
        "char" | "character" | "bpchar" => text_len(args.first().copied()),
        "text" | "citext" | "interval" | "name" | "xml" => long_text(),
        "bytea" => long_binary(),
        "json" | "jsonb" => NeutralType::Json,
        "date" => NeutralType::Date,
        "time" | "time without time zone" => NeutralType::Time {
            precision: args.first().copied(),
            with_tz: false,
        },
        "time with time zone" | "timetz" => NeutralType::Time {
            precision: args.first().copied(),
            with_tz: true,
        },
        "timestamp" | "timestamp without time zone" => NeutralType::Timestamp {
            precision: args.first().copied(),
            with_tz: false,
        },
        "timestamptz" | "timestamp with time zone" => NeutralType::Timestamp {
            precision: args.first().copied(),
            with_tz: true,
        },
        "uuid" => NeutralType::Uuid,
        _ => NeutralType::Unknown,
    }
}

fn sqlite_neutral(parsed: &ParsedType, args: &[u32]) -> NeutralType {
    if parsed.base.contains("int") {
        return NeutralType::Integer {
            bits: 64,
            unsigned: false,
        };
    }
    match parsed.base.as_str() {
        "bool" | "boolean" => NeutralType::Bool,
        "real" | "double" | "float" => NeutralType::Float { bits: 64 },
        "numeric" | "decimal" => NeutralType::Decimal {
            precision: args.first().copied(),
            scale: args.get(1).copied().unwrap_or(0),
        },
        "text" | "varchar" | "char" | "clob" | "nvarchar" | "nchar" | "character" => {
            text_len(args.first().copied())
        }
        "blob" => long_binary(),
        "date" => NeutralType::Date,
        "datetime" | "timestamp" => NeutralType::Timestamp {
            precision: None,
            with_tz: false,
        },
        "time" => NeutralType::Time {
            precision: None,
            with_tz: false,
        },
        "json" => NeutralType::Json,
        _ => NeutralType::Unknown,
    }
}

fn sqlserver_neutral(parsed: &ParsedType, args: &[u32]) -> NeutralType {
    match parsed.base.as_str() {
        "bit" => NeutralType::Bool,
        "tinyint" => NeutralType::Integer {
            bits: 8,
            unsigned: true,
        },
        "smallint" => NeutralType::Integer {
            bits: 16,
            unsigned: false,
        },
        "int" | "integer" => NeutralType::Integer {
            bits: 32,
            unsigned: false,
        },
        "bigint" => NeutralType::Integer {
            bits: 64,
            unsigned: false,
        },
        "decimal" | "numeric" => NeutralType::Decimal {
            precision: args.first().copied(),
            scale: args.get(1).copied().unwrap_or(0),
        },
        "money" => NeutralType::Decimal {
            precision: Some(19),
            scale: 4,
        },
        "smallmoney" => NeutralType::Decimal {
            precision: Some(10),
            scale: 4,
        },
        "real" => NeutralType::Float { bits: 32 },
        "float" => NeutralType::Float { bits: 64 },
        "char" | "varchar" | "nchar" | "nvarchar" => {
            if parsed.args.trim().eq_ignore_ascii_case("max") {
                long_text()
            } else {
                text_len(args.first().copied())
            }
        }
        "text" | "ntext" | "xml" => long_text(),
        "binary" | "varbinary" => {
            if parsed.args.trim().eq_ignore_ascii_case("max") {
                long_binary()
            } else {
                binary_len(args.first().copied())
            }
        }
        "image" => long_binary(),
        "date" => NeutralType::Date,
        "time" => NeutralType::Time {
            precision: args.first().copied(),
            with_tz: false,
        },
        "datetime" | "datetime2" | "smalldatetime" => NeutralType::Timestamp {
            precision: args.first().copied(),
            with_tz: false,
        },
        "datetimeoffset" => NeutralType::Timestamp {
            precision: args.first().copied(),
            with_tz: true,
        },
        "uniqueidentifier" => NeutralType::Uuid,
        _ => NeutralType::Unknown,
    }
}

fn oracle_neutral(parsed: &ParsedType, args: &[u32]) -> NeutralType {
    match parsed.base.as_str() {
        "boolean" => NeutralType::Bool,
        "number" | "decimal" | "numeric" => match (args.first(), args.get(1)) {
            (Some(precision), Some(0)) | (Some(precision), None) if *precision <= 19 => {
                NeutralType::Integer {
                    bits: match precision {
                        0..=2 => 8,
                        3..=4 => 16,
                        5..=9 => 32,
                        _ => 64,
                    },
                    unsigned: false,
                }
            }
            (precision, scale) => NeutralType::Decimal {
                precision: precision.copied(),
                scale: scale.copied().unwrap_or(0),
            },
        },
        "binary_float" => NeutralType::Float { bits: 32 },
        "binary_double" | "float" => NeutralType::Float { bits: 64 },
        "varchar2" | "nvarchar2" | "varchar" | "char" | "nchar" => text_len(args.first().copied()),
        "clob" | "nclob" | "long" | "xmltype" => long_text(),
        "raw" => binary_len(args.first().copied()),
        "blob" | "bfile" | "long raw" => long_binary(),
        "date" => NeutralType::Timestamp {
            precision: None,
            with_tz: false,
        },
        "timestamp" => NeutralType::Timestamp {
            precision: args.first().copied(),
            with_tz: false,
        },
        "timestamp with time zone" | "timestamp with local time zone" => NeutralType::Timestamp {
            precision: args.first().copied(),
            with_tz: true,
        },
        "json" => NeutralType::Json,
        _ => NeutralType::Unknown,
    }
}

fn dameng_neutral(parsed: &ParsedType, args: &[u32]) -> NeutralType {
    match parsed.base.as_str() {
        "bit" | "boolean" | "bool" => NeutralType::Bool,
        "tinyint" => NeutralType::Integer {
            bits: 8,
            unsigned: false,
        },
        "smallint" => NeutralType::Integer {
            bits: 16,
            unsigned: false,
        },
        "int" | "integer" => NeutralType::Integer {
            bits: 32,
            unsigned: false,
        },
        "bigint" => NeutralType::Integer {
            bits: 64,
            unsigned: false,
        },
        "decimal" | "numeric" | "number" => NeutralType::Decimal {
            precision: args.first().copied(),
            scale: args.get(1).copied().unwrap_or(0),
        },
        "real" | "binary_float" => NeutralType::Float { bits: 32 },
        "double" | "double precision" | "binary_double" | "float" => {
            NeutralType::Float { bits: 64 }
        }
        "varchar" | "varchar2" | "char" | "character" | "nchar" | "nvarchar" => {
            text_len(args.first().copied())
        }
        "text" | "clob" | "longvarchar" => long_text(),
        "varbinary" | "binary" | "raw" => binary_len(args.first().copied()),
        "blob" | "image" | "longvarbinary" => long_binary(),
        "date" => NeutralType::Date,
        "time" => NeutralType::Time {
            precision: args.first().copied(),
            with_tz: false,
        },
        "datetime" | "timestamp" => NeutralType::Timestamp {
            precision: args.first().copied(),
            with_tz: false,
        },
        "timestamp with time zone" => NeutralType::Timestamp {
            precision: args.first().copied(),
            with_tz: true,
        },
        "json" => NeutralType::Json,
        _ => NeutralType::Unknown,
    }
}

fn packed(bits: u8) -> &'static str {
    match bits {
        0..=8 => "tinyint",
        9..=16 => "smallint",
        17..=32 => "int",
        _ => "bigint",
    }
}

fn warning_lossy(column_type: &str, target: &str, reason: &str) -> String {
    format!("类型 {column_type} 映射为 {target} 时{reason}")
}

fn clamp(len: u32, max: u32) -> (u32, bool) {
    if len > max { (max, true) } else { (len, false) }
}

/// 按目标方言渲染中性类型。
pub(crate) fn map_type(source: DatabaseKind, target: DatabaseKind, type_text: &str) -> MappedType {
    let neutral = neutral_type(source, type_text);
    render_type(target, &neutral, type_text)
}

fn render_type(target: DatabaseKind, neutral: &NeutralType, original: &str) -> MappedType {
    let mut warnings = Vec::new();
    let text = match target {
        kind if kind.is_mysql_family() => render_mysql(neutral, original, &mut warnings),
        kind if kind.is_postgres_family() => render_postgres(neutral, original, &mut warnings),
        DatabaseKind::Sqlite => render_sqlite(neutral, original, &mut warnings),
        DatabaseKind::SqlServer => render_sqlserver(neutral, original, &mut warnings),
        DatabaseKind::Oracle => render_oracle(neutral, original, &mut warnings),
        DatabaseKind::Dameng => render_dameng(neutral, original, &mut warnings),
        _ => {
            warnings.push(format!("目标数据库不支持自动建表，类型 {original} 未映射"));
            "text".to_string()
        }
    };
    MappedType { text, warnings }
}

fn render_mysql(neutral: &NeutralType, original: &str, warnings: &mut Vec<String>) -> String {
    let unsigned = matches!(neutral, NeutralType::Integer { unsigned: true, .. });
    match neutral {
        NeutralType::Bool => "tinyint(1)".to_string(),
        NeutralType::Integer { bits, .. } => {
            let base = packed(*bits);
            if unsigned {
                format!("{base} unsigned")
            } else {
                base.to_string()
            }
        }
        NeutralType::Decimal { precision, scale } => match precision {
            Some(precision) => format!("decimal({precision},{scale})"),
            None => "decimal".to_string(),
        },
        NeutralType::Float { bits: 32 } => "float".to_string(),
        NeutralType::Float { .. } => "double".to_string(),
        NeutralType::Text { len: Some(len), .. } => {
            let (len, clamped) = clamp(*len, 16_383);
            if clamped {
                warnings.push(warning_lossy(
                    original,
                    "text",
                    "超出 varchar 上限，已改用 text",
                ));
                "text".to_string()
            } else {
                format!("varchar({len})")
            }
        }
        NeutralType::Text { long: true, .. } => "longtext".to_string(),
        NeutralType::Text { .. } => "text".to_string(),
        NeutralType::Binary { len: Some(len), .. } => {
            let (len, clamped) = clamp(*len, 65_535);
            if clamped {
                warnings.push(warning_lossy(
                    original,
                    "longblob",
                    "超出 varbinary 上限，已改用 longblob",
                ));
                "longblob".to_string()
            } else {
                format!("varbinary({len})")
            }
        }
        NeutralType::Binary { .. } => "longblob".to_string(),
        NeutralType::Json => "json".to_string(),
        NeutralType::Date => "date".to_string(),
        NeutralType::Time { precision, .. } => render_with_precision("time", *precision),
        NeutralType::Timestamp { precision, .. } => render_with_precision("datetime", *precision),
        NeutralType::Uuid => "char(36)".to_string(),
        NeutralType::Enum { values } if !values.is_empty() => format!(
            "enum({})",
            values
                .iter()
                .map(|value| format!("'{}'", value.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(",")
        ),
        NeutralType::Enum { .. } => "varchar(255)".to_string(),
        NeutralType::Unknown => {
            warnings.push(warning_lossy(
                original,
                "longtext",
                "无法识别，已改用大文本",
            ));
            "longtext".to_string()
        }
    }
}

fn render_with_precision(base: &str, precision: Option<u32>) -> String {
    match precision.filter(|precision| *precision > 0) {
        Some(precision) => format!("{base}({precision})"),
        None => base.to_string(),
    }
}

fn render_postgres(neutral: &NeutralType, original: &str, warnings: &mut Vec<String>) -> String {
    match neutral {
        NeutralType::Bool => "boolean".to_string(),
        NeutralType::Integer { bits, unsigned } => match (*bits, unsigned) {
            (0..=16, true) => "integer".to_string(),
            (17..=32, true) => "bigint".to_string(),
            // 64 位无符号整数没有 signed 等价物：退回 bigint（与自增列、外键列保持
            // 同一类型），超出 bigint 上限的值会溢出，必须提示。
            (_, true) => {
                warnings.push(warning_lossy(
                    original,
                    "bigint",
                    "PostgreSQL 无无符号整数，已改用 bigint，超出上限的值可能溢出",
                ));
                "bigint".to_string()
            }
            (0..=8, false) | (9..=16, false) => "smallint".to_string(),
            (17..=32, false) => "integer".to_string(),
            _ => "bigint".to_string(),
        },
        NeutralType::Decimal { precision, scale } => match precision {
            Some(precision) => format!("numeric({precision},{scale})"),
            None => "numeric".to_string(),
        },
        NeutralType::Float { bits: 32 } => "real".to_string(),
        NeutralType::Float { .. } => "double precision".to_string(),
        NeutralType::Text { len: Some(len), .. } => {
            let (len, clamped) = clamp(*len, 10_485_760);
            if clamped {
                warnings.push(warning_lossy(
                    original,
                    "text",
                    "超出 varchar 上限，已改用 text",
                ));
                "text".to_string()
            } else {
                format!("varchar({len})")
            }
        }
        NeutralType::Text { .. } => "text".to_string(),
        NeutralType::Binary { long: true, .. } => "bytea".to_string(),
        NeutralType::Binary { .. } => {
            warnings.push(warning_lossy(
                original,
                "bytea",
                "PostgreSQL 不保留二进制长度",
            ));
            "bytea".to_string()
        }
        NeutralType::Json => "jsonb".to_string(),
        NeutralType::Date => "date".to_string(),
        NeutralType::Time { precision, with_tz } => {
            let base = render_with_precision("time", *precision);
            if *with_tz {
                format!("{base} with time zone")
            } else {
                format!("{base} without time zone")
            }
        }
        NeutralType::Timestamp { precision, with_tz } => {
            let base = render_with_precision("timestamp", *precision);
            if *with_tz {
                format!("{base} with time zone")
            } else {
                format!("{base} without time zone")
            }
        }
        NeutralType::Uuid => "uuid".to_string(),
        NeutralType::Enum { .. } => {
            warnings.push(warning_lossy(
                original,
                "text",
                "PostgreSQL 独立枚举类型未自动创建，枚举约束需手工补充",
            ));
            "text".to_string()
        }
        NeutralType::Unknown => {
            warnings.push(warning_lossy(original, "text", "无法识别，已改用文本"));
            "text".to_string()
        }
    }
}

fn render_sqlite(neutral: &NeutralType, original: &str, warnings: &mut Vec<String>) -> String {
    match neutral {
        NeutralType::Bool | NeutralType::Integer { .. } => "integer".to_string(),
        NeutralType::Decimal { .. } | NeutralType::Float { .. } => "numeric".to_string(),
        NeutralType::Text { .. } | NeutralType::Uuid | NeutralType::Enum { .. } => {
            "text".to_string()
        }
        NeutralType::Binary { .. } => "blob".to_string(),
        NeutralType::Json => "text".to_string(),
        NeutralType::Date | NeutralType::Time { .. } | NeutralType::Timestamp { .. } => {
            "text".to_string()
        }
        NeutralType::Unknown => {
            warnings.push(warning_lossy(original, "text", "无法识别，已改用文本"));
            "text".to_string()
        }
    }
}

fn render_sqlserver(neutral: &NeutralType, original: &str, warnings: &mut Vec<String>) -> String {
    match neutral {
        NeutralType::Bool => "bit".to_string(),
        NeutralType::Integer { bits, unsigned } => match (*bits, unsigned) {
            (0..=8, false) => "tinyint".to_string(),
            (0..=8, true) | (9..=16, true) => "smallint".to_string(),
            (9..=16, false) => "smallint".to_string(),
            (17..=32, true) => "bigint".to_string(),
            (17..=32, false) => "int".to_string(),
            (_, true) => {
                warnings.push(warning_lossy(
                    original,
                    "decimal(20,0)",
                    "SQL Server 无无符号大整数，已改用 decimal",
                ));
                "decimal(20,0)".to_string()
            }
            _ => "bigint".to_string(),
        },
        NeutralType::Decimal { precision, scale } => match precision {
            Some(precision) => format!("decimal({precision},{scale})"),
            None => "decimal(18,0)".to_string(),
        },
        NeutralType::Float { bits: 32 } => "real".to_string(),
        NeutralType::Float { .. } => "float".to_string(),
        NeutralType::Text { len: Some(len), .. } => {
            let (len, clamped) = clamp(*len, 4_000);
            if clamped {
                warnings.push(warning_lossy(
                    original,
                    "nvarchar(max)",
                    "超出 nvarchar 上限，已改用 max",
                ));
                "nvarchar(max)".to_string()
            } else {
                format!("nvarchar({len})")
            }
        }
        NeutralType::Text { .. } => "nvarchar(max)".to_string(),
        NeutralType::Binary { len: Some(len), .. } => {
            let (len, clamped) = clamp(*len, 8_000);
            if clamped {
                warnings.push(warning_lossy(
                    original,
                    "varbinary(max)",
                    "超出 varbinary 上限，已改用 max",
                ));
                "varbinary(max)".to_string()
            } else {
                format!("varbinary({len})")
            }
        }
        NeutralType::Binary { .. } => "varbinary(max)".to_string(),
        NeutralType::Json => "nvarchar(max)".to_string(),
        NeutralType::Date => "date".to_string(),
        NeutralType::Time { precision, .. } => render_with_precision("time", *precision),
        NeutralType::Timestamp { precision, with_tz } => {
            let base = if *with_tz {
                "datetimeoffset"
            } else {
                "datetime2"
            };
            render_with_precision(base, *precision)
        }
        NeutralType::Uuid => "uniqueidentifier".to_string(),
        NeutralType::Enum { .. } => {
            warnings.push(warning_lossy(
                original,
                "nvarchar(255)",
                "SQL Server 无枚举类型，枚举约束需手工补充",
            ));
            "nvarchar(255)".to_string()
        }
        NeutralType::Unknown => {
            warnings.push(warning_lossy(
                original,
                "nvarchar(max)",
                "无法识别，已改用大文本",
            ));
            "nvarchar(max)".to_string()
        }
    }
}

fn render_oracle(neutral: &NeutralType, original: &str, warnings: &mut Vec<String>) -> String {
    match neutral {
        NeutralType::Bool => "number(1)".to_string(),
        NeutralType::Integer { bits, unsigned } => match (*bits, unsigned) {
            (0..=8, _) => "number(3)".to_string(),
            (9..=16, _) => "number(5)".to_string(),
            (17..=32, _) => "number(10)".to_string(),
            (_, true) => "number(20)".to_string(),
            _ => "number(19)".to_string(),
        },
        NeutralType::Decimal { precision, scale } => match precision {
            Some(precision) => {
                let (precision, clamped) = clamp(*precision, 38);
                if clamped {
                    warnings.push(warning_lossy(
                        original,
                        "number(38,s)",
                        "精度超过 Oracle 上限，已夹取",
                    ));
                }
                format!("number({precision},{scale})")
            }
            None => "number".to_string(),
        },
        NeutralType::Float { bits: 32 } => "binary_float".to_string(),
        NeutralType::Float { .. } => "binary_double".to_string(),
        NeutralType::Text { len: Some(len), .. } => {
            let (len, clamped) = clamp(*len, 4_000);
            if clamped {
                warnings.push(warning_lossy(
                    original,
                    "clob",
                    "超出 varchar2 上限，已改用 clob",
                ));
                "clob".to_string()
            } else {
                format!("varchar2({len})")
            }
        }
        NeutralType::Text { .. } => "clob".to_string(),
        NeutralType::Binary { len: Some(len), .. } => {
            let (len, clamped) = clamp(*len, 2_000);
            if clamped {
                warnings.push(warning_lossy(
                    original,
                    "blob",
                    "超出 raw 上限，已改用 blob",
                ));
                "blob".to_string()
            } else {
                format!("raw({len})")
            }
        }
        NeutralType::Binary { .. } => "blob".to_string(),
        NeutralType::Json => "clob".to_string(),
        NeutralType::Date => "date".to_string(),
        NeutralType::Time { .. } => {
            warnings.push(warning_lossy(
                original,
                "varchar2(18)",
                "Oracle 无独立 TIME 类型，已改用文本",
            ));
            "varchar2(18)".to_string()
        }
        NeutralType::Timestamp { precision, with_tz } => {
            let base = render_with_precision("timestamp", *precision);
            if *with_tz {
                format!("{base} with time zone")
            } else {
                base
            }
        }
        NeutralType::Uuid => "varchar2(36)".to_string(),
        NeutralType::Enum { .. } => {
            warnings.push(warning_lossy(
                original,
                "varchar2(255)",
                "Oracle 无枚举类型，枚举约束需手工补充",
            ));
            "varchar2(255)".to_string()
        }
        NeutralType::Unknown => {
            warnings.push(warning_lossy(original, "clob", "无法识别，已改用大文本"));
            "clob".to_string()
        }
    }
}

fn render_dameng(neutral: &NeutralType, original: &str, warnings: &mut Vec<String>) -> String {
    match neutral {
        NeutralType::Bool => "bit".to_string(),
        NeutralType::Integer { bits, .. } => packed(*bits).to_string(),
        NeutralType::Decimal { precision, scale } => match precision {
            Some(precision) => format!("decimal({precision},{scale})"),
            None => "decimal".to_string(),
        },
        NeutralType::Float { bits: 32 } => "real".to_string(),
        NeutralType::Float { .. } => "double".to_string(),
        NeutralType::Text { len: Some(len), .. } => {
            let (len, clamped) = clamp(*len, 8_188);
            if clamped {
                warnings.push(warning_lossy(
                    original,
                    "clob",
                    "超出 varchar 上限，已改用 clob",
                ));
                "clob".to_string()
            } else {
                format!("varchar({len})")
            }
        }
        NeutralType::Text { .. } => "clob".to_string(),
        NeutralType::Binary { len: Some(len), .. } => {
            let (len, clamped) = clamp(*len, 8_000);
            if clamped {
                warnings.push(warning_lossy(
                    original,
                    "blob",
                    "超出 varbinary 上限，已改用 blob",
                ));
                "blob".to_string()
            } else {
                format!("varbinary({len})")
            }
        }
        NeutralType::Binary { .. } => "blob".to_string(),
        NeutralType::Json => "clob".to_string(),
        NeutralType::Date => "date".to_string(),
        NeutralType::Time { precision, .. } => render_with_precision("time", *precision),
        NeutralType::Timestamp { precision, with_tz } => {
            let base = render_with_precision("timestamp", *precision);
            if *with_tz {
                format!("{base} with time zone")
            } else {
                base
            }
        }
        NeutralType::Uuid => "char(36)".to_string(),
        NeutralType::Enum { .. } => {
            warnings.push(warning_lossy(
                original,
                "varchar(255)",
                "达梦无枚举类型，枚举约束需手工补充",
            ));
            "varchar(255)".to_string()
        }
        NeutralType::Unknown => {
            warnings.push(warning_lossy(original, "clob", "无法识别，已改用大文本"));
            "clob".to_string()
        }
    }
}

/// 翻译列默认值。只翻译确定等价的表达式，其余丢弃并告警。
pub(crate) fn map_default(
    source: DatabaseKind,
    target: DatabaseKind,
    default_expr: &str,
) -> MappedDefault {
    let raw = default_expr.trim();
    if raw.is_empty() {
        return MappedDefault {
            expr: None,
            warning: None,
        };
    }
    let lower = raw.replace(' ', "").to_ascii_lowercase();
    let function = lower
        .strip_suffix("()")
        .unwrap_or(&lower)
        .trim_matches(|ch| ch == '(' || ch == ')')
        .to_string();

    // 自增序列默认值由 identity 子句承载，不能再作为 DEFAULT 输出。
    if function.starts_with("nextval") || function.contains("nextval(") {
        return MappedDefault {
            expr: None,
            warning: None,
        };
    }

    let current_timestamp = matches!(
        function.as_str(),
        "current_timestamp" | "now" | "getdate" | "sysdatetime" | "localtimestamp"
    );
    let current_date = function.as_str() == "current_date";
    if current_timestamp || current_date {
        // 除 SQL Server 外，目标方言都接受 SQL 标准的 CURRENT_DATE / CURRENT_TIMESTAMP。
        let expr = match target {
            DatabaseKind::SqlServer => {
                if current_date {
                    "CAST(GETDATE() AS date)"
                } else {
                    "GETDATE()"
                }
            }
            _ => {
                if current_date {
                    "CURRENT_DATE"
                } else {
                    "CURRENT_TIMESTAMP"
                }
            }
        };
        return MappedDefault {
            expr: Some(expr.to_string()),
            warning: None,
        };
    }

    // MySQL 的 b'0'/b'1' 是位字面量，落到布尔列时按目标方言改写。
    if let Some(bit) = lower
        .strip_prefix("b'")
        .and_then(|rest| rest.strip_suffix('\''))
    {
        let value = bit == "1";
        let (expr, warning) = match target {
            kind if kind.is_postgres_family() => (value.to_string(), None),
            DatabaseKind::SqlServer => (u8::from(value).to_string(), None),
            DatabaseKind::Oracle | DatabaseKind::Dameng => (u8::from(value).to_string(), None),
            _ => (u8::from(value).to_string(), None),
        };
        return MappedDefault {
            expr: Some(expr),
            warning,
        };
    }

    if lower == "true" || lower == "false" {
        let value = lower == "true";
        let expr = match target {
            kind if kind.is_postgres_family() => value.to_string(),
            _ => u8::from(value).to_string(),
        };
        return MappedDefault {
            expr: Some(expr),
            warning: None,
        };
    }

    // 纯数字、已带引号或 SQL 标准 NULL 直接沿用。
    let is_number = raw.parse::<f64>().is_ok();
    let is_quoted = raw.starts_with('\'') || raw.starts_with('"') || raw.starts_with("N'");
    if is_number || is_quoted || lower == "null" {
        let expr = if is_quoted {
            normalize_string_literal(raw, target)
        } else {
            raw.to_string()
        };
        return MappedDefault {
            expr: Some(expr),
            warning: None,
        };
    }

    // SQL Server 默认约束常带外层括号。
    if raw.starts_with('(') && raw.ends_with(')') {
        return map_default(source, target, raw[1..raw.len() - 1].trim());
    }

    // MySQL 的 column_default 对字符串返回裸文本（没有引号），落到其它方言要补引号。
    if source.is_mysql_family() && !raw.contains('(') {
        let value = raw
            .trim_matches('\'')
            .replace("\\'", "'")
            .replace("''", "'");
        return MappedDefault {
            expr: Some(string_literal(&value, target)),
            warning: None,
        };
    }

    MappedDefault {
        expr: None,
        warning: Some(format!(
            "默认值 {raw} 依赖源数据库函数或表达式，未自动迁移，请手工补充"
        )),
    }
}

fn normalize_string_literal(raw: &str, target: DatabaseKind) -> String {
    match quoted_literal_value(raw) {
        Some(value) => string_literal(&value.replace("\\'", "'"), target),
        None => raw.to_string(),
    }
}

/// 取出字符串字面量的值，忽略 PostgreSQL 的 `::type` 尾缀。
fn quoted_literal_value(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let body = trimmed
        .strip_prefix("N'")
        .or_else(|| trimmed.strip_prefix('\''))?;
    let chars = body.chars().collect::<Vec<_>>();
    let mut value = String::new();
    let mut index = 0;
    while index < chars.len() {
        // MySQL 用反斜杠转义引号，SQL 标准用双写；两种都接受。
        if chars[index] == '\\' && chars.get(index + 1) == Some(&'\'') {
            value.push('\'');
            index += 2;
            continue;
        }
        if chars[index] == '\'' {
            if chars.get(index + 1) == Some(&'\'') {
                value.push('\'');
                index += 2;
                continue;
            }
            return Some(value);
        }
        value.push(chars[index]);
        index += 1;
    }
    None
}

fn string_literal(value: &str, target: DatabaseKind) -> String {
    if target == DatabaseKind::SqlServer {
        format!("N'{}'", value.replace('\'', "''"))
    } else if target.is_mysql_family() {
        format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapped(source: DatabaseKind, target: DatabaseKind, text: &str) -> String {
        map_type(source, target, text).text
    }

    #[test]
    fn mysql_types_map_to_postgres() {
        assert_eq!(
            mapped(DatabaseKind::MySql, DatabaseKind::Postgres, "int"),
            "integer"
        );
        assert_eq!(
            mapped(
                DatabaseKind::MySql,
                DatabaseKind::Postgres,
                "bigint unsigned"
            ),
            "bigint"
        );
        assert_eq!(
            mapped(DatabaseKind::MySql, DatabaseKind::Postgres, "varchar(64)"),
            "varchar(64)"
        );
        assert_eq!(
            mapped(DatabaseKind::MySql, DatabaseKind::Postgres, "datetime(3)"),
            "timestamp(3) without time zone"
        );
        assert_eq!(
            mapped(DatabaseKind::MySql, DatabaseKind::Postgres, "datetime"),
            "timestamp without time zone"
        );
        assert_eq!(
            mapped(DatabaseKind::MySql, DatabaseKind::Postgres, "tinyint(1)"),
            "boolean"
        );
        assert_eq!(
            mapped(DatabaseKind::MySql, DatabaseKind::Postgres, "enum('a','b')"),
            "text"
        );
    }

    #[test]
    fn postgres_types_map_to_mysql() {
        assert_eq!(
            mapped(
                DatabaseKind::Postgres,
                DatabaseKind::MySql,
                "timestamp with time zone"
            ),
            "datetime"
        );
        assert_eq!(
            mapped(DatabaseKind::Postgres, DatabaseKind::MySql, "uuid"),
            "char(36)"
        );
        assert_eq!(
            mapped(DatabaseKind::Postgres, DatabaseKind::MySql, "numeric(10,2)"),
            "decimal(10,2)"
        );
        assert_eq!(
            mapped(DatabaseKind::Postgres, DatabaseKind::MySql, "text"),
            "longtext"
        );
    }

    #[test]
    fn oracle_number_and_sqlserver_identity_types() {
        assert_eq!(
            mapped(DatabaseKind::Oracle, DatabaseKind::Postgres, "number(10)"),
            "bigint"
        );
        assert_eq!(
            mapped(DatabaseKind::Oracle, DatabaseKind::Postgres, "number(19)"),
            "bigint"
        );
        assert_eq!(
            mapped(DatabaseKind::Oracle, DatabaseKind::Postgres, "number(20)"),
            "numeric(20,0)"
        );
        assert_eq!(
            mapped(DatabaseKind::Oracle, DatabaseKind::Postgres, "number(10,2)"),
            "numeric(10,2)"
        );
        assert_eq!(
            mapped(
                DatabaseKind::SqlServer,
                DatabaseKind::MySql,
                "nvarchar(max)"
            ),
            "longtext"
        );
        assert_eq!(
            mapped(DatabaseKind::SqlServer, DatabaseKind::MySql, "datetime2(7)"),
            "datetime(7)"
        );
    }

    #[test]
    fn long_text_clamps_with_warning() {
        let result = map_type(DatabaseKind::MySql, DatabaseKind::Oracle, "varchar(8000)");
        assert_eq!(result.text, "clob");
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn defaults_translate_known_functions_and_drop_unknown() {
        let now = map_default(DatabaseKind::MySql, DatabaseKind::SqlServer, "now()");
        assert_eq!(now.expr.as_deref(), Some("GETDATE()"));

        let next = map_default(
            DatabaseKind::Postgres,
            DatabaseKind::MySql,
            "nextval('orders_id_seq'::regclass)",
        );
        assert!(next.expr.is_none());

        let quoted = map_default(DatabaseKind::MySql, DatabaseKind::Postgres, "it\\'s");
        assert_eq!(quoted.expr.as_deref(), Some("'it''s'"));

        let unknown = map_default(
            DatabaseKind::Postgres,
            DatabaseKind::MySql,
            "gen_random_uuid()",
        );
        assert!(unknown.expr.is_none());
        assert!(unknown.warning.is_some());

        // PostgreSQL 默认值常带类型转换，比较与迁移都要剥掉。
        let cast = map_default(
            DatabaseKind::Postgres,
            DatabaseKind::MySql,
            "'pending'::character varying",
        );
        assert_eq!(cast.expr.as_deref(), Some("'pending'"));
    }
}
