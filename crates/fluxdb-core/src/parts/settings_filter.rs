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

/// 「默认分页行数」设置项的可选档位（升序）。数据表页大小只能取这些值。
pub const DATA_TABLE_PAGE_SIZE_CHOICES: [(&str, u64); 3] = [("100", 100), ("500", 500), ("1000", 1000)];

/// 数据表页大小 / 数据页 SQL 面板里 LIMIT 的封顶值 —— 取可选档位里最大的一档。
///
/// 刻意从档位表推导而不是另写一个数：这两处表达的是**同一个产品上限**。
/// 历史上它们各写各的（档位到 1000，面板封顶却是写死的 100），导致「默认分页行数」
/// 选了 500/1000 后，一按刷新（⌘R）就被面板的上限夹回 100。
pub fn data_table_page_size_max() -> u64 {
    DATA_TABLE_PAGE_SIZE_CHOICES
        .iter()
        .map(|(_, value)| *value)
        .max()
        .unwrap_or(Pagination::DEFAULT_LIMIT)
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

/// 查询结果区的停靠位置。SQL 编辑器的查询结果面板与 Redis Workbench 的结果区
/// 共用同一个值，两处始终一致。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResultsPlacement {
    /// 结果区在编辑器下方（上下分栏）。
    Bottom = 0,
    /// 结果区在编辑器右侧（左右分栏）。
    Right = 1,
}

impl ResultsPlacement {
    /// 在两种布局之间翻转（工具栏切换按钮用）。
    pub fn toggled(self) -> Self {
        match self {
            Self::Bottom => Self::Right,
            Self::Right => Self::Bottom,
        }
    }

    /// 设置面板的选择控件只吃 `u64`，这里给出稳定下标（等于上面的判别值）。
    /// `const` 是为了让面板的 `&'static` 选项表能在编译期算出来。
    pub const fn to_index(self) -> u64 {
        self as u64
    }

    /// `to_index` 的逆运算；越界时回退到默认值，避免脏配置让面板无按钮高亮。
    pub fn from_index(index: u64) -> Self {
        match index {
            1 => Self::Right,
            _ => Self::Bottom,
        }
    }
}

impl Default for ResultsPlacement {
    fn default() -> Self {
        Self::Bottom
    }
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
    /// SQL 查询结果的每页行数（0 表示不限制）。仅作用于查询编辑器执行结果，
    /// 数据表浏览的页大小见 `data_table_page_size`。
    pub page_size: u64,
    /// 新打开数据表时的默认每页加载行数。仅作用于数据表编辑器，与查询结果的
    /// `page_size` 相互独立；取值经 `Pagination::new` 收敛到合法区间。
    #[serde(default = "default_data_table_page_size")]
    pub data_table_page_size: u64,
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
    /// 危险 SQL 操作清单（"哪些语句算危险"）。集合项见 `DangerousSqlAction` 的 key，
    /// 空集合 = 不把任何 SQL 判为危险。仅影响 SQL 侧，Redis 清单不受影响。
    #[serde(default = "default_dangerous_sql_actions")]
    pub dangerous_sql_actions: BTreeSet<String>,
    #[serde(default = "default_completion_index_enabled")]
    pub enable_completion_index: bool,
    /// Redis Workbench 编辑器区（上方）相对分栏可用高度的占比（0-100），用于记住并恢复分栏大小。
    /// 用整数百分比而非 f32，保持 `Settings` 结构体可派生 `Eq`。
    #[serde(default = "default_redis_workbench_editor_ratio")]
    pub redis_workbench_editor_ratio: u8,
    /// 查询结果区的默认停靠位置（下方 / 右侧）。同时驱动 SQL 编辑器的查询结果面板
    /// 与 Redis Workbench 的结果区；工具栏上的切换按钮只改运行时值，不改这里。
    #[serde(default)]
    pub results_placement: ResultsPlacement,
    /// Redis Workbench 在「右侧」布局下编辑器区的宽度（像素）。
    /// 与 `redis_workbench_editor_ratio` 相互独立：两种布局各自记住自己的分栏大小，
    /// 切换布局不会把另一种布局的尺寸带偏。
    #[serde(default = "default_redis_workbench_editor_width")]
    pub redis_workbench_editor_width: u16,
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
            data_table_page_size: default_data_table_page_size(),
            show_sidebar: true,
            show_inspector: true,
            editor_font_size: default_editor_font_size(),
            editor_line_height: default_editor_line_height(),
            editor_tab_width: default_editor_tab_width(),
            editor_word_wrap: false,
            confirm_dangerous_sql: default_dangerous_sql_confirmation(),
            confirm_dangerous_redis: default_dangerous_sql_confirmation(),
            dangerous_sql_actions: default_dangerous_sql_actions(),
            enable_completion_index: default_completion_index_enabled(),
            redis_workbench_editor_ratio: default_redis_workbench_editor_ratio(),
            results_placement: ResultsPlacement::default(),
            redis_workbench_editor_width: default_redis_workbench_editor_width(),
            custom_keybindings: BTreeMap::new(),
            backup_dir: String::new(),
            mysqldump_path: String::new(),
            sqlite3_path: String::new(),
        }
    }
}

/// 可配置危险 SQL 操作的预置项。`key` 为持久化到 `Settings.dangerous_sql_actions`
/// 集合的稳定标识（小写下划线）；`title`/`description` 用于设置面板勾选清单展示。
pub struct DangerousSqlAction {
    pub key: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// 默认是否勾选（即该项默认算不算危险）。
    pub default_checked: bool,
}

impl DangerousSqlAction {
    /// 所有预置项。前四项默认勾选（保持 `is_dangerous_sql_statement` 现状行为），
    /// `alter_table` 默认关，避免误伤普通 DDL。
    pub const ALL: [DangerousSqlAction; 5] = [
        DangerousSqlAction {
            key: "drop",
            title: "DROP",
            description: "删除表或数据库",
            default_checked: true,
        },
        DangerousSqlAction {
            key: "truncate",
            title: "TRUNCATE",
            description: "清空表数据",
            default_checked: true,
        },
        DangerousSqlAction {
            key: "update_without_where",
            title: "UPDATE 无 WHERE",
            description: "更新语句未带 WHERE 条件",
            default_checked: true,
        },
        DangerousSqlAction {
            key: "delete_without_where",
            title: "DELETE 无 WHERE",
            description: "删除语句未带 WHERE 条件",
            default_checked: true,
        },
        DangerousSqlAction {
            key: "alter_table",
            title: "ALTER TABLE",
            description: "修改表结构",
            default_checked: false,
        },
    ];
}

fn default_editor_font_size() -> u32 {
    12
}

/// 数据表默认页大小。默认与 `Pagination::DEFAULT_LIMIT` 对齐，
/// 老配置文件缺该字段时按此值补齐。
fn default_data_table_page_size() -> u64 {
    Pagination::DEFAULT_LIMIT
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

/// 默认危险 SQL 操作清单：保持为现行行为（DROP/TRUNCATE/无 WHERE 的 UPDATE/DELETE），
/// 不含 `alter_table`（用户需手动勾选）。
fn default_dangerous_sql_actions() -> BTreeSet<String> {
    DangerousSqlAction::ALL
        .iter()
        .filter(|a| a.default_checked)
        .map(|a| a.key.to_string())
        .collect()
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

/// Redis Workbench 右侧布局下编辑器区的默认宽度（像素）。
/// 取固定像素而非占比：宽度基准要减掉连接侧边栏（宽达 180-600px），
/// 按视口宽算占比会整体偏，且拖拽会被百分比量化成十几像素一跳。
/// 与 SQL 查询结果面板的 `QUERY_OUTPUT_DEFAULT_WIDTH` 同一套模型，行为保持一致。
fn default_redis_workbench_editor_width() -> u16 {
    720
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
