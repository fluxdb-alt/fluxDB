#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataPage {
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    pub offset: u64,
    pub limit: u64,
    pub has_more: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub type_name: Option<String>,
    pub nullable: bool,
    pub primary_key: bool,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub is_unique: bool,
    pub is_primary: bool,
    pub index_type: Option<String>,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForeignKeyInfo {
    pub name: String,
    pub column: String,
    pub ref_schema: Option<String>,
    pub ref_table: String,
    pub ref_column: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TriggerInfo {
    pub name: String,
    pub event: String,
    pub timing: String,
    pub body: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub values: Vec<CellValue>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BinaryCellSummary {
    pub type_name: String,
    pub is_null: bool,
    pub byte_length: u64,
    pub preview_hex: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryPreviewResponse {
    pub byte_length: u64,
    pub preview_hex: String,
    pub preview_size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BinaryUpdatePayload {
    SetNull,
    Hex(String),
    FilePath(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CellValue {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    Text(String),
    Bytes(Vec<u8>),
    BinarySummary(BinaryCellSummary),
    Json(String),
}

impl CellValue {
    pub fn display_label(&self) -> String {
        match self {
            CellValue::Null => "NULL".to_string(),
            CellValue::Bool(value) => value.to_string(),
            CellValue::I64(value) => value.to_string(),
            CellValue::F64(value) => value.to_string(),
            CellValue::Text(value) => value.clone(),
            CellValue::Bytes(value) => format!("(BLOB) {} bytes", value.len()),
            CellValue::BinarySummary(summary) => {
                if summary.is_null {
                    "NULL".to_string()
                } else {
                    format!(
                        "{} [{}]",
                        summary.type_name.to_ascii_uppercase(),
                        format_byte_length(summary.byte_length)
                    )
                }
            }
            CellValue::Json(value) => value.clone(),
        }
    }
}

pub fn is_binary_type_name(type_name: &str) -> bool {
    let Some(base) = type_name.trim().split('(').next() else {
        return false;
    };
    let normalized = base
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();

    matches!(
        normalized.as_str(),
        "binary"
            | "varbinary"
            | "tinyblob"
            | "blob"
            | "mediumblob"
            | "longblob"
            | "bytea"
            | "image"
            | "raw"
            | "long raw"
    )
}

pub fn binary_type_max_bytes(type_name: &str) -> Option<u64> {
    let normalized = type_name.trim().to_ascii_lowercase();
    let base = normalized
        .split('(')
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    match base.as_str() {
        "binary" | "varbinary" | "raw" => binary_type_length_arg(&normalized),
        "tinyblob" => Some(255),
        "blob" => Some(65_535),
        "mediumblob" => Some(16_777_215),
        "longblob" => Some(4_294_967_295),
        _ => None,
    }
}

fn binary_type_length_arg(type_name: &str) -> Option<u64> {
    let start = type_name.find('(')?;
    let end = type_name[start + 1..].find(')')? + start + 1;
    type_name[start + 1..end].trim().parse().ok()
}

pub fn format_byte_length(byte_length: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const TB: f64 = 1024.0 * 1024.0 * 1024.0 * 1024.0;

    match byte_length {
        0..=1023 => format!("{byte_length} B"),
        1024..=1_048_575 => format!("{:.1} KB", byte_length as f64 / KB),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", byte_length as f64 / MB),
        1_073_741_824..=1_099_511_627_775 => format!("{:.1} GB", byte_length as f64 / GB),
        _ => format!("{:.1} TB", byte_length as f64 / TB),
    }
}
