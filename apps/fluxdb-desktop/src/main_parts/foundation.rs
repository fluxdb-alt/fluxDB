// ---------------------------------------------------------------
// crate 根作用域共享导入 / 常量 / 工具函数。
// 旧 `sql_editor/model.rs` / `sql_editor/layout.rs` 经 include! 在 crate 根提供了
// 下列 gpui / std 导入、`EDITOR_FONT` 与 `rgba_with_alpha`，被终端、建表、菜单等模块
// 无前缀引用。旧 sql_editor 模块移除后，在此原样重建以保持其余模块可解析。
// ---------------------------------------------------------------
use std::ops::Range;
use gpui::{
    Action, ElementInputHandler, GlobalElementId, LayoutId, TextRun, UTF16Selection, fill,
};
// SQL 语句运行相关类型（`SqlStatementRun` / `SqlStatementId` / `SqlStatementStatus`）
// 已从旧 sql_editor/model.rs 迁移到 sql_editor_adapter；此处在 crate 根统一引入，
// 使宿主各模块可如旧版一样无前缀使用。
use sql_editor_adapter::{SqlStatementId, SqlStatementRun, SqlStatementStatus};

/// 查询编辑器等复用的等宽字体名（原定义于旧 sql_editor/layout.rs）。
const EDITOR_FONT: &str = "Menlo";

/// 给 RGBA 颜色覆写 alpha 通道（原定义于旧 sql_editor/layout.rs，被菜单/弹窗等复用）。
fn rgba_with_alpha(mut color: gpui::Rgba, alpha: f32) -> gpui::Rgba {
    color.a = alpha;
    color
}

/// 顶部栏 GitHub 按钮打开的项目地址。改仓库时只改这一处。
const GITHUB_REPOSITORY_URL: &str = "https://github.com/fluxdb-alt/fluxDB";
const DEFAULT_CONNECTION_COLOR: &str = "#202124";
const CONNECTION_COLOR_OPTION: &str = "color";
const VISIBLE_DATABASES_OPTION: &str = "visible_databases";
const CONNECTION_BROWSER_DEFAULT_WIDTH: f32 = 250.;
const CONNECTION_BROWSER_MIN_WIDTH: f32 = 180.;
const CONNECTION_BROWSER_MAX_WIDTH: f32 = 600.;
const DATA_FILTER_PANEL_MIN_HEIGHT: f32 = 126.;
const DATA_FILTER_PANEL_MAX_HEIGHT: f32 = 420.;
const TABLE_INFO_MIN_WIDTH: f32 = 260.;
const TABLE_INFO_MAX_WIDTH: f32 = 800.;
const CELL_DETAIL_DRAWER_DEFAULT_HEIGHT: f32 = 160.;
const CELL_DETAIL_DRAWER_MIN_HEIGHT: f32 = 128.;
const CELL_DETAIL_DRAWER_MAX_HEIGHT: f32 = 360.;
const SQL_HIGHLIGHT_LANGUAGE: &str = "sql";
const MYSQL_DDL_HIGHLIGHT_LANGUAGE: &str = "mysql-ddl";
const JSON_HIGHLIGHT_LANGUAGE: &str = "json";
/// 底部提示（`app_message_overlay`）的自动消失时长。
const APP_MESSAGE_DURATION: Duration = Duration::from_millis(3000);
/// 底部提示的行高。gpui-component 的默认行高对 14px 小字号偏松，提示条会显得很占地方。
const APP_MESSAGE_LINE_HEIGHT: f32 = 18.;
/// 页面级错误块（`page_error_alert`）的行高：要承载整段原始报文，比提示条略松一点。
const PAGE_ERROR_LINE_HEIGHT: f32 = 20.;
/// 告警淡色底的混色比例：变体色占多少，其余为面板底色。
/// 太大文字会被底色吃掉，太小则看不出是一块提示，0.12~0.16 是常见区间。
const ALERT_TINT: f32 = 0.14;
const TREE_ARROW_COL_WIDTH: f32 = 18.;
const TREE_ICON_COL_WIDTH: f32 = 22.;
const CONNECTION_COLOR_PALETTE: &[(&str, u32)] = &[
    ("#202124", 0x202124),
    ("#12c95b", 0x12c95b),
    ("#fbbc04", 0xfbbc04),
    ("#ff7a00", 0xff7a00),
    ("#ff3b45", 0xff3b45),
    ("#3478f6", 0x3478f6),
    ("#a142f4", 0xa142f4),
];
#[derive(Clone, Copy)]
struct UiColors {
    is_dark: bool,
    app_bg: gpui::Rgba,
    content_bg: gpui::Rgba,
    sidebar_bg: gpui::Rgba,
    panel_bg: gpui::Rgba,
    panel_alt: gpui::Rgba,
    border: gpui::Rgba,
    border_soft: gpui::Rgba,
    text: gpui::Rgba,
    muted: gpui::Rgba,
    tree_bg: gpui::Rgba,
    tree_selected: gpui::Rgba,
    hover: gpui::Rgba,
    input_bg: gpui::Rgba,
    status_bg: gpui::Rgba,
    // 圆角风格：跟随 gpui-component theme.radius / radius_lg，按钮/输入框/面板共用。
    radius: gpui::Pixels,
    radius_lg: gpui::Pixels,
}

fn ui_colors_from_theme(mode: ThemeMode, cx: &App) -> UiColors {
    let theme = ComponentTheme::global(cx);
    let mut colors = ui_colors(mode);
    // 主题注册表是唯一配色来源；旧版自定义颜色字段仅保留反序列化兼容。
    colors.app_bg = theme.title_bar.into();
    colors.content_bg = theme.background.into();
    colors.sidebar_bg = theme.sidebar.into();
    colors.panel_bg = theme.background.into();
    colors.panel_alt = theme.list_head.into();
    colors.border = theme.border.into();
    colors.border_soft = theme.border.into();
    colors.text = theme.foreground.into();
    colors.muted = theme.muted_foreground.into();
    colors.tree_bg = theme.sidebar.into();
    colors.tree_selected = theme.list_active.into();
    colors.hover = theme.list_hover.into();
    colors.input_bg = theme.background.into();
    colors.status_bg = theme.tab_bar.into();
    // 圆角风格设置注入：按钮/输入框/面板等表面统一跟随主题半径。
    colors.radius = theme.radius;
    colors.radius_lg = theme.radius_lg;
    colors
}

fn apply_component_theme_colors(mode: ThemeMode, cx: &mut App) {
    let colors = ui_colors_from_theme(mode, cx);
    let accent_hsla = ComponentTheme::global(cx).primary;
    let component_colors = &mut ComponentTheme::global_mut(cx).colors;

    component_colors.background = colors.panel_bg.into();
    component_colors.foreground = colors.text.into();
    component_colors.border = colors.border.into();
    component_colors.input = colors.border.into();
    component_colors.ring = accent_hsla;
    component_colors.caret = accent_hsla;

    component_colors.muted = colors.panel_alt.into();
    component_colors.muted_foreground = colors.muted.into();
    component_colors.popover = colors.panel_bg.into();
    component_colors.popover_foreground = colors.text.into();

    component_colors.primary = accent_hsla;
    component_colors.primary_hover = accent_hsla.opacity(0.88);
    component_colors.primary_active = accent_hsla.opacity(0.78);
    component_colors.primary_foreground = rgb(0xffffff).into();
    component_colors.accent = accent_hsla;
    component_colors.accent_foreground = rgb(0xffffff).into();

    component_colors.list = colors.panel_bg.into();
    component_colors.list_head = colors.panel_alt.into();
    component_colors.list_hover = colors.hover.into();
    component_colors.list_active = colors.tree_selected.into();
    component_colors.list_active_border = accent_hsla;

    component_colors.sidebar = colors.sidebar_bg.into();
    component_colors.sidebar_border = colors.border.into();
    component_colors.sidebar_foreground = colors.text.into();
    component_colors.sidebar_accent = colors.tree_selected.into();
    component_colors.sidebar_accent_foreground = colors.text.into();
    component_colors.sidebar_primary = accent_hsla;
    component_colors.sidebar_primary_foreground = rgb(0xffffff).into();

    component_colors.table = colors.panel_bg.into();
    component_colors.table_head = colors.panel_alt.into();
    component_colors.table_head_foreground = colors.muted.into();
    component_colors.table_hover = colors.hover.into();
    component_colors.table_active = colors.tree_selected.into();
    component_colors.table_active_border = accent_hsla;
    component_colors.table_row_border = colors.border_soft.into();

    component_colors.tab = colors.panel_bg.into();
    component_colors.tab_active = colors.panel_bg.into();
    component_colors.tab_active_foreground = colors.text.into();
    component_colors.tab_bar = colors.panel_alt.into();
    component_colors.tab_bar_segmented = colors.panel_alt.into();
    component_colors.tab_foreground = colors.muted.into();

    // 让 code_editor（当前仅 JSON）的 token 配色对齐 JsonEditorTheme：
    // string 红 / number 绿 / boolean 蓝 / key 紫 / 标点中性，避免编辑态出现 string 变绿的错位。
    apply_json_highlight_theme(mode, cx);
}

fn apply_button_radius(radius: u8, cx: &mut App) {
    let radius = f32::from(radius.clamp(0, 24));
    ComponentTheme::global_mut(cx).radius = px(radius);
    ComponentTheme::sync_base(cx);
}

fn apply_global_theme_settings(settings: &Settings, cx: &mut App) {
    use gpui_component::scroll::ScrollbarMode as ComponentScrollbarMode;
    use std::time::Duration;

    let theme = ComponentTheme::global_mut(cx);
    theme.radius_lg = px(f32::from(settings.large_radius.clamp(0, 32)));
    theme.shadow = settings.show_shadows;
    theme.focus_ring = settings.focus_ring;
    theme.scrollbar_mode = match settings.scrollbar_mode {
        ScrollbarMode::Scrolling => ComponentScrollbarMode::Scrolling,
        ScrollbarMode::Hover => ComponentScrollbarMode::Hover,
        ScrollbarMode::Always => ComponentScrollbarMode::Always,
    };
    theme.font_family = if settings.global_font_family.trim().is_empty() {
        ".SystemUIFont".into()
    } else {
        settings.global_font_family.trim().into()
    };
    theme.font_size = px(match settings.ui_density {
        UiDensity::Compact => 15.,
        UiDensity::Standard => 16.,
        UiDensity::Comfortable => 17.,
    });
    if settings.reduce_motion {
        theme.motion.duration_fast = Duration::ZERO;
        theme.motion.duration_normal = Duration::ZERO;
        theme.motion.duration_slow = Duration::ZERO;
    } else {
        let defaults = gpui_component::MotionTokens::default();
        theme.motion.duration_fast = defaults.duration_fast;
        theme.motion.duration_normal = defaults.duration_normal;
        theme.motion.duration_slow = defaults.duration_slow;
    }
    ComponentTheme::sync_base(cx);
}

/// 覆盖全局 `HighlightTheme` 的 JSON token 色，使编辑态高亮与查看态 `JsonEditorTheme` 一致。
///
/// 默认 Zed 主题把 string 染成绿色、number 染成蓝色，与 Redis JSON 值展示明显不符；
/// 当前应用内所有 `InputState::code_editor` 均为 JSON，因此全局覆盖不会影响其它语言。
fn apply_json_highlight_theme(mode: ThemeMode, cx: &mut App) {
    use gpui_component::highlighter::HighlightTheme;
    let (appearance, syntax) = match mode {
        ThemeMode::Dark => (
            "dark",
            r##"{"string":{"color":"#ff7b9c"},"number":{"color":"#7ee787"},"boolean":{"color":"#79c0ff"},"constant":{"color":"#79c0ff"},"string.special":{"color":"#bb9aff"},"comment":{"color":"#8b949e"},"punctuation":{"color":"#8b949e"},"string.escape":{"color":"#8b949e"}}"##,
        ),
        ThemeMode::Light => (
            "light",
            r##"{"string":{"color":"#c7377e"},"number":{"color":"#1a7f37"},"boolean":{"color":"#0969da"},"constant":{"color":"#0969da"},"string.special":{"color":"#7d5bb0"},"comment":{"color":"#57606a"},"punctuation":{"color":"#57606a"},"string.escape":{"color":"#57606a"}}"##,
        ),
    };
    let json = format!(
        r#"{{"name":"gdb-json-highlight","appearance":"{}","style":{{"syntax":{}}}}}"#,
        appearance, syntax
    );
    if let Ok(theme) = serde_json::from_str::<HighlightTheme>(&json) {
        ComponentTheme::global_mut(cx).highlight_theme = std::sync::Arc::new(theme);
    }
}

fn parse_theme_hex_color(hex: &str) -> Option<gpui::Rgba> {
    parse_hex_rgb(hex).map(rgb)
}

fn normalize_theme_hex_color(hex: &str) -> Option<String> {
    let value = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
    if value.len() != 6 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", value.to_ascii_uppercase()))
}

fn parse_hex_rgb(hex: &str) -> Option<u32> {
    let hex = normalize_theme_hex_color(hex)?;
    u32::from_str_radix(&hex[1..], 16).ok()
}

fn ui_colors(mode: ThemeMode) -> UiColors {
    if mode == ThemeMode::Dark {
        UiColors {
            is_dark: true,
            app_bg: rgb(0x101214),
            content_bg: rgb(0x121417),
            sidebar_bg: rgb(0x171a1f),
            panel_bg: rgb(0x1b1f25),
            panel_alt: rgb(0x232832),
            border: rgb(0x343a44),
            border_soft: rgb(0x2a3038),
            text: rgb(0xe6e8ec),
            muted: rgb(0x9aa3af),
            tree_bg: rgb(0x171a1f),
            tree_selected: rgb(0x2a313b),
            hover: rgb(0x252b34),
            input_bg: rgb(0x111418),
            status_bg: rgb(0x15181c),
            radius: px(6.),
            radius_lg: px(8.),
        }
    } else {
        UiColors {
            is_dark: false,
            app_bg: rgb(0xf5f5f5),
            content_bg: rgb(0xf6f7f9),
            sidebar_bg: rgb(0xf2f3f5),
            panel_bg: rgb(0xffffff),
            panel_alt: rgb(0xf4f6f8),
            border: rgb(0xd4d8de),
            border_soft: rgb(0xe5e9ef),
            text: rgb(0x20242a),
            muted: rgb(0x737985),
            tree_bg: rgb(0xf2f3f5),
            tree_selected: rgb(0xd9e1e7),
            hover: rgb(0xe9edf3),
            input_bg: rgb(0xffffff),
            status_bg: rgb(0xf0f0f0),
            radius: px(6.),
            radius_lg: px(8.),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppIcon {
    Activity,
    AlignLeft,
    ArrowUpDown,
    Bot,
    Broadcast,
    Check,
    CalendarClock,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    ChevronsLeft,
    ChevronsRight,
    CircleSlash,
    Close,
    Copy,
    Database,
    Edit,
    Eye,
    EyeOff,
    FileSearch,
    FileSql,
    Filter,
    Folder,
    FolderInput,
    FolderUp,
    Github,
    Home,
    List,
    Maximize,
    Minus,
    PanelBottom,
    PanelLeftClose,
    PanelLeftOpen,
    PanelRight,
    Pin,
    Play,
    Plug,
    Plus,
    Query,
    Refresh,
    Redo,
    Search,
    #[allow(dead_code)] // 经典图标资源，供后续“选择”类工具使用，保留。
    Select,
    Settings,
    Save,
    Square,
    Table,
    Trash,
    Undo,
    Users,
    Wand,
    Workflow,
    Text,
    WrapText,
}

fn app_icon_path(icon: AppIcon) -> &'static str {
    match icon {
        AppIcon::Activity => "icons/activity.svg",
        AppIcon::AlignLeft => "icons/align-left.svg",
        AppIcon::ArrowUpDown => "icons/arrow-up-down.svg",
        AppIcon::Bot => "icons/bot.svg",
        AppIcon::Broadcast => "icons/broadcast.svg",
        AppIcon::Check => "icons/check.svg",
        AppIcon::CalendarClock => "icons/calendar-clock.svg",
        AppIcon::ChevronDown => "icons/chevron-down.svg",
        AppIcon::ChevronLeft => "icons/chevron-left.svg",
        AppIcon::ChevronRight => "icons/chevron-right.svg",
        AppIcon::ChevronUp => "icons/chevron-up.svg",
        AppIcon::ChevronsLeft => "icons/chevrons-left.svg",
        AppIcon::ChevronsRight => "icons/chevrons-right.svg",
        AppIcon::CircleSlash => "icons/circle-slash.svg",
        AppIcon::Close => "icons/x.svg",
        AppIcon::Copy => "icons/copy-plus.svg",
        AppIcon::Database => "icons/database.svg",
        AppIcon::Edit => "icons/pencil.svg",
        AppIcon::Eye => "icons/eye.svg",
        AppIcon::EyeOff => "icons/eye-off.svg",
        AppIcon::FileSearch => "icons/file-search.svg",
        AppIcon::FileSql => "icons/file-code.svg",
        AppIcon::Filter => "icons/list-filter.svg",
        AppIcon::Folder => "icons/folder.svg",
        AppIcon::FolderInput => "icons/folder-input.svg",
        AppIcon::FolderUp => "icons/folder-up.svg",
        AppIcon::Github => "icons/github.svg",
        AppIcon::Home => "icons/home.svg",
        AppIcon::List => "icons/list.svg",
        AppIcon::Maximize => "icons/maximize-2.svg",
        AppIcon::Minus => "icons/minus.svg",
        AppIcon::PanelBottom => "icons/panel-bottom.svg",
        AppIcon::PanelLeftClose => "icons/panel-left-close.svg",
        AppIcon::PanelLeftOpen => "icons/panel-left-open.svg",
        AppIcon::PanelRight => "icons/panel-right.svg",
        AppIcon::Pin => "icons/pin.svg",
        AppIcon::Play => "icons/play.svg",
        AppIcon::Plug => "icons/plug.svg",
        AppIcon::Plus => "icons/plus.svg",
        AppIcon::Query => "icons/terminal-square.svg",
        AppIcon::Refresh => "icons/refresh-cw.svg",
        AppIcon::Redo => "icons/redo-2.svg",
        AppIcon::Search => "icons/search.svg",
        AppIcon::Select => "icons/text-select.svg",
        AppIcon::Settings => "icons/settings.svg",
        AppIcon::Save => "icons/save.svg",
        AppIcon::Square => "icons/square.svg",
        AppIcon::Table => "icons/table-2.svg",
        AppIcon::Trash => "icons/trash-2.svg",
        AppIcon::Undo => "icons/undo-2.svg",
        AppIcon::Users => "icons/users.svg",
        AppIcon::Wand => "icons/wand-sparkles.svg",
        AppIcon::Workflow => "icons/workflow.svg",
        AppIcon::Text => "icons/text.svg",
        AppIcon::WrapText => "icons/wrap-text.svg",
    }
}

fn app_icon(icon: AppIcon, size: f32, color: gpui::Rgba) -> gpui::AnyElement {
    svg()
        .size(px(size))
        .path(app_icon_path(icon))
        .text_color(color)
        .into_any_element()
}

fn app_icon_box(icon: AppIcon, box_size: f32, icon_size: f32, color: gpui::Rgba) -> Div {
    div()
        .size(px(box_size))
        .flex()
        .items_center()
        .justify_center()
        .child(app_icon(icon, icon_size, color))
}

actions!(
    gdb,
    [
        NewQuery,
        Quit,
        Refresh,
        SaveOrApply,
        CloseCurrentTab,
        ToggleConnectionBrowser,
        ExecuteOrApply,
        OpenDataSearch,
        OpenQueryHistoryQuickSearch,
        QueryHistoryQuickSearchPrevious,
        QueryHistoryQuickSearchNext,
        QueryHistoryQuickSearchConfirm,
        CopyDataSelection,
        CopyFooterSqlSelection,
        DeleteConnectionShortcut,
        CancelDeleteConnection,
        CancelDialog,
        OpenNewConnection,
        OpenSettingsMenu,
        OpenQueryHistory,
        ToggleTheme
    ]
);

struct Assets {
    base: PathBuf,
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        let asset_path = self.base.join(path);
        fs::read(&asset_path)
            .map(Cow::Owned)
            .map(Some)
            .map_err(|error| anyhow::anyhow!("读取资源 {} 失败: {}", asset_path.display(), error))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(fs::read_dir(self.base.join(path))?
            .filter_map(|entry| {
                entry
                    .ok()
                    .and_then(|entry| entry.file_name().into_string().ok())
                    .map(SharedString::from)
            })
            .collect())
    }
}
