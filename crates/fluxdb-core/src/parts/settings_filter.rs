#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Pagination {
    pub offset: u64,
    pub limit: u64,
}

impl Pagination {
    pub const DEFAULT_LIMIT: u64 = 100;
    /// 单页最大行数上限。Redis Key 列表的 Folder / Scan more 需要允许累计超过单批 10000，
    /// 所以这里把上限放宽到一个更大的值，避免分页在中途被硬截断。
    pub const MAX_LIMIT: u64 = 100_000;

    pub fn new(offset: u64, limit: u64) -> Self {
        Self {
            offset,
            limit: limit.clamp(1, Self::MAX_LIMIT),
        }
    }
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: Self::DEFAULT_LIMIT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScrollbarMode {
    Scrolling,
    Hover,
    Always,
}

impl Default for ScrollbarMode {
    fn default() -> Self {
        Self::Scrolling
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiDensity {
    Compact,
    Standard,
    Comfortable,
}

impl Default for UiDensity {
    fn default() -> Self {
        Self::Standard
    }
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Info
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub theme: Theme,
    #[serde(default)]
    pub log_level: LogLevel,
    #[serde(default)]
    pub log_path: String,
    #[serde(default = "default_button_radius")]
    pub button_radius: u8,
    #[serde(default = "default_large_radius")]
    pub large_radius: u8,
    #[serde(default = "default_true")]
    pub show_shadows: bool,
    #[serde(default = "default_true")]
    pub focus_ring: bool,
    #[serde(default)]
    pub scrollbar_mode: ScrollbarMode,
    #[serde(default)]
    pub ui_density: UiDensity,
    #[serde(default = "default_true")]
    pub show_status_bar: bool,
    #[serde(default)]
    pub reduce_motion: bool,
    /// 系统设置-性能诊断：是否显示 FPS / 帧耗时 / CPU / GPU / 内存 悬浮 HUD。
    #[serde(default)]
    pub performance_diagnostics: bool,
    #[serde(default)]
    pub global_font_family: String,
    /// Selected light/dark palette names from the application theme registry.
    #[serde(default = "default_light_theme")]
    pub light_theme: String,
    #[serde(default = "default_dark_theme")]
    pub dark_theme: String,
    pub page_size: u64,
    pub show_sidebar: bool,
    pub show_inspector: bool,
    #[serde(default = "default_editor_font_size")]
    pub editor_font_size: u32,
    #[serde(default = "default_editor_line_height")]
    pub editor_line_height: u32,
    #[serde(default = "default_editor_tab_width")]
    pub editor_tab_width: u32,
    #[serde(default)]
    pub editor_word_wrap: bool,
    #[serde(default = "default_dangerous_sql_confirmation")]
    pub confirm_dangerous_sql: bool,
    /// Redis Workbench 执行破坏性命令（FLUSHDB/FLUSHALL 等）前是否二次确认。
    #[serde(default = "default_dangerous_sql_confirmation")]
    pub confirm_dangerous_redis: bool,
    #[serde(default = "default_completion_index_enabled")]
    pub enable_completion_index: bool,
    /// Redis Workbench 编辑器区（上方）相对分栏可用高度的占比（0-100），用于记住并恢复分栏大小。
    /// 用整数百分比而非 f32，保持 `Settings` 结构体可派生 `Eq`。
    #[serde(default = "default_redis_workbench_editor_ratio")]
    pub redis_workbench_editor_ratio: u8,
    /// 用户自定义的应用级快捷键，键为稳定 action id，值为 GPUI keystroke 规范。
    #[serde(default)]
    pub custom_keybindings: BTreeMap<String, String>,
    /// 数据库备份文件的默认保存目录，为空时回退到系统下载目录。
    #[serde(default)]
    pub backup_dir: String,
    /// 原生备份工具 mysqldump 的路径；为空时从系统 PATH 查找。
    #[serde(default)]
    pub mysqldump_path: String,
    /// 原生备份工具 sqlite3 的路径；为空时从系统 PATH 查找。
    #[serde(default)]
    pub sqlite3_path: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            log_level: LogLevel::default(),
            log_path: String::new(),
            button_radius: default_button_radius(),
            large_radius: default_large_radius(),
            show_shadows: true,
            focus_ring: true,
            scrollbar_mode: ScrollbarMode::default(),
            ui_density: UiDensity::default(),
            show_status_bar: true,
            reduce_motion: false,
            performance_diagnostics: false,
            global_font_family: String::new(),
            light_theme: default_light_theme(),
            dark_theme: default_dark_theme(),
            page_size: Pagination::DEFAULT_LIMIT,
            show_sidebar: true,
            show_inspector: true,
            editor_font_size: default_editor_font_size(),
            editor_line_height: default_editor_line_height(),
            editor_tab_width: default_editor_tab_width(),
            editor_word_wrap: false,
            confirm_dangerous_sql: default_dangerous_sql_confirmation(),
            confirm_dangerous_redis: default_dangerous_sql_confirmation(),
            enable_completion_index: default_completion_index_enabled(),
            redis_workbench_editor_ratio: default_redis_workbench_editor_ratio(),
            custom_keybindings: BTreeMap::new(),
            backup_dir: String::new(),
            mysqldump_path: String::new(),
            sqlite3_path: String::new(),
        }
    }
}

fn default_editor_font_size() -> u32 {
    12
}

fn default_editor_line_height() -> u32 {
    14
}

fn default_editor_tab_width() -> u32 {
    4
}

fn default_dangerous_sql_confirmation() -> bool {
    true
}

fn default_completion_index_enabled() -> bool {
    true
}

fn default_light_theme() -> String {
    "Default Light".to_string()
}

fn default_dark_theme() -> String {
    "Default Dark".to_string()
}

fn default_redis_workbench_editor_ratio() -> u8 {
    // 结果区默认占分栏 37.5% ⇒ editor 占分栏 ≈ 62.5%。
    63
}

fn default_button_radius() -> u8 {
    6
}

fn default_large_radius() -> u8 {
    8
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SortSpec {
    pub field: String,
    pub direction: SortDirection,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FilterSpec {
    pub field: String,
    pub op: FilterOp,
    pub values: Vec<CellValue>,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterOp {
    Eq,
    NotEq,
    Contains,
    NotContains,
    StartsWith,
    NotStartsWith,
    EndsWith,
    NotEndsWith,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Between,
    NotBetween,
    InList,
    NotInList,
    IsNull,
    IsNotNull,
    IsEmpty,
    IsNotEmpty,
    Exists,
    NotExists,
}
