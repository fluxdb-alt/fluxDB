use std::path::Path;

/// 导入写入方式。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TableImportMode {
    /// 追加到目标表现有数据之后（默认）。
    #[default]
    Append,
    /// 先清空目标表再写入。MySQL/达梦的 TRUNCATE 无法回滚，调用方必须提示风险。
    Truncate,
}

impl TableImportMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Append => "追加",
            Self::Truncate => "清空后导入",
        }
    }
}

/// 默认每批写入行数：与历史行为一致，避免参数绑定数量失控。
pub const TABLE_IMPORT_DEFAULT_BATCH_ROWS: usize = 500;

/// 文件解析选项；留空字段表示使用该格式的默认值。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TableImportOptions {
    /// 文本编码名（如 "utf-8"、"gb18030"）。空表示按 UTF-8 解析。
    pub encoding: String,
    /// 分隔符；仅 TXT 生效。空表示 Tab。
    pub delimiter: String,
    /// 引号符；仅 TXT/CSV 生效。空表示 `"`。
    pub quote: String,
    /// 首行是否为表头。JSON/XML 忽略该选项。
    pub has_header: bool,
    /// 是否去除字段首尾空白。
    pub trim_whitespace: bool,
    /// 空字符串是否按 NULL 写入。
    pub empty_as_null: bool,
    /// 导入模式，默认追加。
    pub mode: TableImportMode,
    /// 每批写入行数；0 使用默认值 500。批越大参数越多，需低于目标库上限。
    pub batch_rows: usize,
}

impl TableImportOptions {
    /// 默认值：首行为表头、不去空白、空串按文本、追加写入。
    pub fn new() -> Self {
        Self {
            has_header: true,
            ..Self::default()
        }
    }

    pub fn delimiter_byte(&self) -> u8 {
        self.delimiter
            .as_bytes()
            .first()
            .copied()
            .unwrap_or(b'\t')
    }

    /// 实际使用的每批行数；0 或未设置时回退到默认 500。
    pub fn rows_per_batch(&self) -> usize {
        if self.batch_rows == 0 {
            TABLE_IMPORT_DEFAULT_BATCH_ROWS
        } else {
            self.batch_rows
        }
    }

    pub fn quote_byte(&self) -> u8 {
        self.quote.as_bytes().first().copied().unwrap_or(b'"')
    }
}

/// 与“导出数据”保持同一格式集合；SQL 导出不作为导入来源。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TableImportFormat {
    Txt,
    Csv,
    Json,
    Xml,
}

impl TableImportFormat {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("txt") => Some(Self::Txt),
            Some("csv") => Some(Self::Csv),
            Some("json") => Some(Self::Json),
            Some("xml") => Some(Self::Xml),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Txt => "TXT",
            Self::Csv => "CSV",
            Self::Json => "JSON",
            Self::Xml => "XML",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableImportRequest {
    pub config: ConnectionConfig,
    pub object: ObjectPath,
    pub source: PathBuf,
    pub format: TableImportFormat,
    pub options: TableImportOptions,
    /// 源列名 → 目标列名；为空表示按同名自动匹配。
    pub column_mapping: Vec<TableImportColumnMapping>,
}

/// 单列映射。`target` 为 `None` 表示跳过该源列。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableImportColumnMapping {
    pub source: String,
    pub target: Option<String>,
}

/// 目标表字段信息，供映射界面展示。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableImportTargetColumn {
    pub name: String,
    pub type_name: Option<String>,
    pub nullable: bool,
}

/// 导入前预览：只解析有限行，不写数据库。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableImportPreview {
    pub format: String,
    pub encoding: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// 预览只读取前若干行；真实行数需完整解析后才能确定。
    pub truncated: bool,
    pub target_columns: Vec<TableImportTargetColumn>,
}

/// 预览请求只依赖连接配置、文件与解析选项，不需要表对象。
#[derive(Clone, Debug, PartialEq)]
pub struct TableImportPreviewRequest {
    pub config: ConnectionConfig,
    pub object: ObjectPath,
    pub source: PathBuf,
    pub format: TableImportFormat,
    pub options: TableImportOptions,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TableImportReport {
    pub total_rows: u64,
    pub imported_rows: u64,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableImportProgress {
    pub stage: String,
    pub message: String,
}

/// 表数据导入的统一能力边界。
///
/// 实现负责解析文件、校验目标列并按数据库方言执行单条参数化 INSERT；
/// 上层只依赖该 trait，不直接拼 SQL 或处理数据库差异。
pub trait TableDataImport: Send + Sync {
    fn kind(&self) -> DatabaseKind;
    fn import(
        &self,
        request: &TableImportRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(TableImportProgress),
    ) -> crate::Result<TableImportReport>;

    /// 解析文件样本并返回目标表字段；不写数据库。
    fn preview(
        &self,
        request: &TableImportPreviewRequest,
    ) -> crate::Result<TableImportPreview>;
}
