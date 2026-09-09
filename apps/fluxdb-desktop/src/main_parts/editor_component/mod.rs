// editor_component/mod.rs —— 通用 GPUI 编辑器前端核心。
//
// 该模块基于 fluxdb-editor-core 的纯逻辑内核（EditorBuffer / Selection / DisplayMap /
// CompletionController 等），并提供 GPUI 渲染、输入（IME）、软换行、行号、折叠、
// 以及 completion / hover / hint / diagnostic / decoration 等 provider 协议的接入。
//
// 分层约束（AGENTS.md / 设计文档）：
//   - 本模块不依赖 fluxdb-app / connector / 数据库驱动 / NavicatMain；
//     connection / database / schema 只会通过 provider 注入到“适配器层”。
//   - 通用编辑能力（增删改、光标、选择、缩进、补全列表渲染）都在此处实现，
//     不含任何业务类型。

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeSet, HashMap},
    ops::Range,
    sync::{atomic::{AtomicU64, Ordering}, Arc},
    time::{Duration, Instant},
};

use fluxdb_editor_core::{
    Block, BlockId, BufferSnapshot, CancellationToken, CodeLens, CodeLensProvider, CompletionContinuation, CompletionController, CompletionItem, CompletionProvider, CompletionRequest, CompletionSession, DecorationProvider, Diagnostic, DiagnosticProvider, DisplayMap, DocumentationRequest, DocumentationState, EditorBuffer,
    EditorConfig, EditorProfile, ExecuteMode, ExecutionAdapter, Fold, FoldSet, HoverContent, HoverProvider, InlineHintProvider,
    inline_hints_to_snapshot, InputEdit, InsertTextFormat, LanguageDefinition, LanguageRegistry,
    Point, Range as CoreRange, Selection,
    SignatureInfo, SignatureProvider, SoftWrap, SyntaxLayerTree, SyntaxProvider, SyntaxSnapshot,
    TaskOutcome, TextChange, TriggerDecision,
};

// 事件协议统一由内核提供；这里把 `EditorEvent` 作为本模块公共路径再导出，
// 供宿主（NavicatMain 等）以 `editor_component::EditorEvent` 匹配订阅事件。
pub(crate) use fluxdb_editor_core::EditorEvent;

use gpui::{
    actions, px, Action, App, Bounds, ContentMask, Context, Element, ElementId, ElementInputHandler, EventEmitter,
    AppContext, FocusHandle, GlobalElementId, IntoElement, InteractiveElement, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render, ScrollHandle, ScrollDelta, Div,
    ScrollWheelEvent, SharedString, Size, Subscription, Task, TextAlign, TextRun, TruncateFrom,
    UTF16Selection,
    ParentElement, ShapedLine, Stateful, StatefulInteractiveElement, Styled, StyledText, Window, div, hsla,
    svg,
};
use gpui_component::{Sizable, box_shadow};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::scroll::{ScrollableElement, Scrollbar};

/// GPUI 像素坐标点（与内核 `Point` 区分）。
pub(crate) type GPoint = gpui::Point<Pixels>;

// ---------------------------------------------------------------- 常量

pub(crate) const EDITOR_PADDING_X: f32 = 10.;
pub(crate) const EDITOR_PADDING_Y: f32 = 4.;
pub(crate) const EDITOR_LINE_GAP: f32 = 0.;
pub(crate) const EDITOR_GUTTER_GAP: f32 = 10.;
pub(crate) const EDITOR_CONTENT_GAP: f32 = 6.;
pub(crate) const EDITOR_MIN_GUTTER: f32 = 44.;
pub(crate) const EDITOR_FOLD_GUTTER: f32 = 16.;
pub(crate) const EDITOR_FONT: &str = "Menlo";
pub(crate) const EDITOR_TEXT_SIZE: f32 = 12.;
pub(crate) const COMPLETION_POPUP_MIN_WIDTH: f32 = 280.;
pub(crate) const COMPLETION_POPUP_MAX_WIDTH: f32 = 540.;
pub(crate) const COMPLETION_ROW_HEIGHT: f32 = 30.;
/// 补全框最小高度：候选过少（1~2 条）时框体仍保持可读高度，避免缩成一条细带。
/// 约 3 行（3×行高 + 内边距/边框 10px）。
pub(crate) const COMPLETION_POPUP_MIN_HEIGHT: f32 = 3. * COMPLETION_ROW_HEIGHT + 10.;

pub(crate) const CODE_LENS_HEIGHT: f32 = 14.;
/// 附加语言能力的统一防抖窗口；文本和光标更新不受影响。
const PROVIDER_DEBOUNCE_MS: u64 = 75;
const LARGE_DOCUMENT_DIAGNOSTIC_DEBOUNCE_MS: u64 = 500;
/// 大文档编辑时不搬移整份 token 数组；渲染层会立刻使用可见区 fallback。
static NEXT_EDITOR_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_EDIT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);

/// 滚动趋近阻尼系数（1/s）：控制向目标偏移指数趋近的快慢。越大越跟手、
/// 越快收敛；实测 60Hz 下取 50 约 5 帧（~80ms）趋近完成，丝滑且无明显惯性余量。
const SCROLL_EASE_K: f32 = 50.0;
/// 滚动停止判定阈值（px）：距目标小于该值视为静止，结束动画时钟。
const SCROLL_STOP_DISTANCE: f32 = 0.5;

struct ContentWidthCache {
    buffer_version: u64,
    text_len: usize,
    line_count: usize,
    font_size_bits: u32,
    width: f32,
}

struct FindMatchesCache {
    version: u64,
    query: String,
    match_case: bool,
    whole_word: bool,
    regex: bool,
    matches: Arc<Vec<CoreRange>>,
}

// ---------------------------------------------------------------- 事件

// 事件协议统一复用内核 `fluxdb_editor_core::EditorEvent`，不再维护本地重复枚举：
//   - Changed(TextChange) 携带增量变更（旧区间 + 新文本 + 版本），宿主据此做增量同步，
//     不再退化为逐按键全文 `Editor::text()`。
//   - Execute{range, mode} 只传递中性标识，SQL 语义与语句文本由接入层（SqlAdapter/宿主）解释。
//   - SelectionChanged / CompletionAccepted 等与内核变体一一对应。
impl EventEmitter<EditorEvent> for Editor {}

// ---------------------------------------------------------------- 输入动作

#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = editor_component, no_json)]
pub(crate) struct Enter {
    pub secondary: bool,
}

actions!(
    editor_component,
    [
        Backspace,
        Delete,
        IndentInline,
        OutdentInline,
        MoveUp,
        MoveDown,
        MoveLeft,
        MoveRight,
        MovePageUp,
        MovePageDown,
        MoveHome,
        MoveEnd,
        MoveToStart,
        MoveToEnd,
        MoveToPreviousWord,
        MoveToNextWord,
        SelectAll,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectHome,
        SelectEnd,
        SelectToStart,
        SelectToEnd,
        SelectToPreviousWord,
        SelectToNextWord,
        SelectPageUp,
        SelectPageDown,
        SelectLine,
        Undo,
        Redo,
        Copy,
        Cut,
        Paste,
        TriggerCompletion,
        ToggleFold,
        FoldAll,
        UnfoldAll,
        OpenFind,
        CloseFind,
        FindNext,
        FindPrevious,
        ToggleLineComment,
        Escape
    ]
);

pub(crate) const CONTEXT: &str = "EditorComponent";

pub(crate) fn register_editor_shortcuts(cx: &mut App) {
    // 与旧 sql_editor 相同的绑定方式：KeyBinding::new 泛型接收具体动作结构体。
    cx.bind_keys(vec![
        KeyBinding::new("backspace", Backspace, Some(CONTEXT)),
        KeyBinding::new("delete", Delete, Some(CONTEXT)),
        KeyBinding::new("enter", Enter { secondary: false }, Some(CONTEXT)),
        KeyBinding::new("secondary-enter", Enter { secondary: true }, Some(CONTEXT)),
        KeyBinding::new("escape", Escape, Some(CONTEXT)),
        KeyBinding::new("up", MoveUp, Some(CONTEXT)),
        KeyBinding::new("down", MoveDown, Some(CONTEXT)),
        KeyBinding::new("left", MoveLeft, Some(CONTEXT)),
        KeyBinding::new("right", MoveRight, Some(CONTEXT)),
        KeyBinding::new("pageup", MovePageUp, Some(CONTEXT)),
        KeyBinding::new("pagedown", MovePageDown, Some(CONTEXT)),
        KeyBinding::new("home", MoveHome, Some(CONTEXT)),
        KeyBinding::new("end", MoveEnd, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-left", MoveHome, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-right", MoveEnd, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-up", MoveToStart, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-down", MoveToEnd, Some(CONTEXT)),
        KeyBinding::new("shift-left", SelectLeft, Some(CONTEXT)),
        KeyBinding::new("shift-right", SelectRight, Some(CONTEXT)),
        KeyBinding::new("shift-up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("shift-down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("shift-home", SelectHome, Some(CONTEXT)),
        KeyBinding::new("shift-end", SelectEnd, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("shift-cmd-left", SelectHome, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("shift-cmd-right", SelectEnd, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("shift-cmd-up", SelectToStart, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("shift-cmd-down", SelectToEnd, Some(CONTEXT)),
        KeyBinding::new("shift-pageup", SelectPageUp, Some(CONTEXT)),
        KeyBinding::new("shift-pagedown", SelectPageDown, Some(CONTEXT)),
        KeyBinding::new("tab", IndentInline, Some(CONTEXT)),
        KeyBinding::new("shift-tab", OutdentInline, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-a", SelectAll, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-a", SelectAll, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-c", Copy, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-c", Copy, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-x", Cut, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-x", Cut, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-v", Paste, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-v", Paste, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-z", Undo, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-z", Undo, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("shift-cmd-z", Redo, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-y", Redo, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-f", OpenFind, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-/", ToggleLineComment, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-/", ToggleLineComment, Some(CONTEXT)),
        KeyBinding::new("ctrl-space", TriggerCompletion, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-]", ToggleFold, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-{", FoldAll, Some(CONTEXT)),
        KeyBinding::new("cmd-alt-{", UnfoldAll, Some(CONTEXT)),
    ]);
}

// ---------------------------------------------------------------- Provider 容器

/// 编辑器可选的 provider。使用 `Option<Arc<dyn ...>>` 允许编辑器在无 provider 时仍能工作
/// （纯编辑），由接入层（SQL 适配器）注入具体实现。
// `syntax` / `hover` / `signature` / `inline_hint` / `execution` 为接入层插槽；
// CodeLens 与 decorations 由布局/渲染阶段消费。
#[allow(dead_code)]
pub(crate) struct Providers {
    /// 按 `EditorProfile::language_id` 解析 language/syntax；显式 provider 优先。
    pub language_registry: Option<Arc<LanguageRegistry>>,
    pub language: Option<Arc<dyn LanguageDefinition>>,
    pub syntax: Option<Arc<dyn SyntaxProvider>>,
    pub completion: Option<Arc<dyn CompletionProvider>>,
    pub hover: Option<Arc<dyn HoverProvider>>,
    pub signature: Option<Arc<dyn SignatureProvider>>,
    pub inline_hint: Option<Arc<dyn InlineHintProvider>>,
    pub diagnostics: Option<Arc<dyn DiagnosticProvider>>,
    pub decorations: Option<Arc<dyn DecorationProvider>>,
    pub code_lens: Option<Arc<dyn CodeLensProvider>>,
    pub execution: Option<Arc<dyn ExecutionAdapter>>,
}

impl Default for Providers {
    fn default() -> Self {
        Self {
            language_registry: None,
            language: None,
            syntax: None,
            completion: None,
            hover: None,
            signature: None,
            inline_hint: None,
            diagnostics: None,
            decorations: None,
            code_lens: None,
            execution: None,
        }
    }
}

// ---------------------------------------------------------------- 行命中区域

/// 行级命中区域（折叠箭头 / 运行按钮等），由布局阶段填充，供输入层命中检测。
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct LineHitRegion {
    pub row: usize,
    pub bounds: gpui::Bounds<gpui::Pixels>,
    pub kind: LineHitKind,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineHitKind {
    Fold,
    Run,
    Select,
    Explain,
    Hover,
    CodeLens(usize),
}

#[derive(Clone, Debug)]
pub(crate) struct CodeLensHit {
    pub range: CoreRange,
    pub action: String,
}

// ---------------------------------------------------------------- 查找状态

/// 编辑器内的查找状态（通用能力，宿主渲染 hosting 面板并读写本状态）。
pub(crate) struct FindState {
    /// 查找面板是否打开。
    pub open: bool,
    /// 当前查找词。
    pub query: String,
    /// 替换词。
    #[allow(dead_code)] // 查找替换面板尚未接入宿主渲染，字段保留以待接线。
    pub replace_text: String,
    /// 是否区分大小写。
    pub match_case: bool,
    /// 是否整词匹配。
    pub whole_word: bool,
    /// 是否正则匹配。
    pub regex: bool,
    /// 光标所在是否命中查找词（供面板高亮）。
    pub found: bool,
    /// 从当前光标向后/向前查找时使用的“已消费”标记，避免在同一位置无限循环。
    #[allow(dead_code)] // 同上，查找替换流程暂未接入宿主。
    search_start: usize,
    /// 最近一次命中区间（字节），供替换使用。
    pub current_match: Option<CoreRange>,
}

/// 查找选项种类。
#[allow(dead_code)] // 查找选项面板暂未接入，保留枚举与查找面板能力。
pub(crate) enum FindOption {
    MatchCase,
    WholeWord,
    Regex,
}

/// 判断 [start, end) 区间两侧是否都是词边界（用于整词匹配）。
fn is_word_boundary(text: &str, start: usize, end: usize) -> bool {
    let bytes = text.as_bytes();
    let is_word = |i: usize| -> bool {
        bytes
            .get(i)
            .map(|b| b.is_ascii_alphanumeric() || *b == b'_')
            .unwrap_or(false)
    };
    let left_ok = start == 0 || !is_word(start - 1);
    let right_ok = end >= bytes.len() || !is_word(end);
    left_ok && right_ok
}

impl Default for FindState {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            replace_text: String::new(),
            match_case: false,
            whole_word: false,
            regex: false,
            found: false,
            search_start: 0,
            current_match: None,
        }
    }
}

// ---------------------------------------------------------------- 编辑器主题

/// 编辑器配色集合（整改设计 4.2：可注入的 `EditorTheme`）。
///
/// 正文 / 光标 / 选择 / 当前行 / gutter / popup / 诊断 / syntax token 全部从这里读取；
/// 明亮与暗黑两套由宿主依据 `ThemeMode`/`UiColors` 选择注入，`Default`（暗黑）仅作为
/// 宿主尚未注入时的兜底。编辑器模块本身不依赖宿主主题系统，保持内核通用。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EditorTheme {
    pub background: gpui::Rgba,
    pub text: gpui::Rgba,
    pub active_line: gpui::Rgba,
    pub selection: gpui::Rgba,
    pub cursor: gpui::Rgba,
    pub line_number: gpui::Rgba,
    pub error: gpui::Rgba,
    pub warning: gpui::Rgba,
    pub completion_bg: gpui::Rgba,
    pub completion_selected_bg: gpui::Rgba,
    pub completion_text: gpui::Rgba,
    pub completion_detail: gpui::Rgba,
    pub completion_highlight: gpui::Rgba,
    pub status_running: gpui::Rgba,
    pub status_success: gpui::Rgba,
    pub status_failure: gpui::Rgba,
    pub syntax_keyword: gpui::Rgba,
    pub syntax_string: gpui::Rgba,
    pub syntax_number: gpui::Rgba,
    pub syntax_comment: gpui::Rgba,
    pub syntax_type: gpui::Rgba,
    pub syntax_identifier: gpui::Rgba,
    pub syntax_field: gpui::Rgba,
    pub syntax_function: gpui::Rgba,
    pub syntax_attribute: gpui::Rgba,
    pub syntax_variable: gpui::Rgba,
    pub syntax_parameter: gpui::Rgba,
    pub syntax_boolean: gpui::Rgba,
}

impl EditorTheme {
    /// 语法元素 kind → 前景色；未知 kind 返回 `None`（由调用方回退到默认文本色）。
    pub(crate) fn syntax_color(&self, kind: &str) -> Option<gpui::Rgba> {
        let c = match kind {
            "keyword" => self.syntax_keyword,
            "string" => self.syntax_string,
            "number" => self.syntax_number,
            "comment" => self.syntax_comment,
            "type" => self.syntax_type,
            "identifier" => self.syntax_identifier,
            "field" => self.syntax_field,
            "function" => self.syntax_function,
            "attribute" => self.syntax_attribute,
            "variable" => self.syntax_variable,
            "parameter" => self.syntax_parameter,
            "boolean" => self.syntax_boolean,
            _ => return None,
        };
        Some(c)
    }

    /// 暗黑主题：深色 SQL 工作台配色，突出关键字、标识符与字面量层次。
    pub(crate) fn dark() -> Self {
        Self {
            background: gpui::rgb(0x1e1e1e),
            text: gpui::rgb(0xd4d4d4),
            active_line: gpui::rgb(0x2a2a2a),
            selection: gpui::rgb(0x264f78),
            cursor: gpui::rgb(0xaeafad),
            line_number: gpui::rgb(0x6a6a6a),
            error: gpui::rgb(0xf14c4c),
            warning: gpui::rgb(0xd7ba7d),
            completion_bg: gpui::rgb(0x252526),
            completion_selected_bg: gpui::rgb(0x04395e),
            completion_text: gpui::rgb(0xd4d4d4),
            completion_detail: gpui::rgb(0x9d9d9d),
            completion_highlight: gpui::rgb(0xe2c08d),
            // 语句执行状态：行背景用低饱和色调，避免喧宾夺主。
            status_running: gpui::rgb(0x2a3a52),
            status_success: gpui::rgb(0x1f3a2a),
            status_failure: gpui::rgb(0x4a2626),
            // 语法 token：蓝色关键字/类型、橙色标识符、紫色数字、红色字符串。
            syntax_keyword: gpui::rgb(0x569cd6),
            syntax_string: gpui::rgb(0xd87979),
            syntax_number: gpui::rgb(0xb58cff),
            syntax_comment: gpui::rgb(0x7f9f7f),
            syntax_type: gpui::rgb(0x569cd6),
            syntax_identifier: gpui::rgb(0xffb52e),
            // 字段名与表名/索引名保持同一标识符色，贴近主流 SQL 编辑器的语义。
            syntax_field: gpui::rgb(0xffb52e),
            syntax_function: gpui::rgb(0xdcdcaa),
            syntax_attribute: gpui::rgb(0xc586c0),
            syntax_variable: gpui::rgb(0x4ec9b0),
            syntax_parameter: gpui::rgb(0x9cdcfe),
            syntax_boolean: gpui::rgb(0x569cd6),
        }
    }

    /// 明亮主题（VS Code Light+ 近似）。宿主在 `ThemeMode::Light` 下注入。
    pub(crate) fn light() -> Self {
        Self {
            background: gpui::rgb(0xffffff),
            text: gpui::rgb(0x1f1f1f),
            active_line: gpui::rgb(0xf5f5f5),
            selection: gpui::rgb(0xadd6ff),
            cursor: gpui::rgb(0x333333),
            line_number: gpui::rgb(0x9e9e9e),
            error: gpui::rgb(0xf14c4c),
            warning: gpui::rgb(0xc69026),
            completion_bg: gpui::rgb(0xf7f7f7),
            completion_selected_bg: gpui::rgb(0xdbeafe),
            completion_text: gpui::rgb(0x1f1f1f),
            completion_detail: gpui::rgb(0x707070),
            completion_highlight: gpui::rgb(0x795e26),
            // 语句执行状态：浅色下行背景，保持与前文一致的低饱和策略。
            status_running: gpui::rgb(0xd6e6f7),
            status_success: gpui::rgb(0xdcf0e3),
            status_failure: gpui::rgb(0xf7dede),
            // 语法 token（VS Code Light+ 近似，与 foundation 的 JSON 浅色语系一致风格）。
            syntax_keyword: gpui::rgb(0x0000ff),
            syntax_string: gpui::rgb(0xa31515),
            syntax_number: gpui::rgb(0x098658),
            syntax_comment: gpui::rgb(0x008000),
            syntax_type: gpui::rgb(0x267f99),
            syntax_identifier: gpui::rgb(0x795e26),
            syntax_field: gpui::rgb(0x795e26),
            syntax_function: gpui::rgb(0x795e26),
            syntax_attribute: gpui::rgb(0x0000ff),
            syntax_variable: gpui::rgb(0x267f99),
            syntax_parameter: gpui::rgb(0x0451a5),
            syntax_boolean: gpui::rgb(0x0451a5),
        }
    }
}

impl Default for EditorTheme {
    fn default() -> Self {
        Self::dark()
    }
}

#[cfg(test)]
mod theme_tests {
    use super::*;

    /// 计算 WCAG 相对亮度（0..=1）。
    fn luminance(c: gpui::Rgba) -> f64 {
        let channel = |v: f32| {
            let v = v as f64;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let r = channel(c.r);
        let g = channel(c.g);
        let b = channel(c.b);
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// 两组颜色的对比度（1..=21）。
    fn contrast(a: gpui::Rgba, b: gpui::Rgba) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// 主题下「正文相对背景」与「popup 文本相对 popup 背景」的对比度均需可读。
    #[test]
    fn both_themes_text_is_readable_over_background() {
        for theme in [EditorTheme::dark(), EditorTheme::light()] {
            // 正文相对编辑区背景。
            assert!(
                contrast(theme.text, theme.background) >= 7.0,
                "text/background 对比度过低: {:?}",
                contrast(theme.text, theme.background)
            );
            // popup 文本相对 popup 背景。
            assert!(
                contrast(theme.completion_text, theme.completion_bg) >= 4.5,
                "completion/background 对比度过低: {:?}",
                contrast(theme.completion_text, theme.completion_bg)
            );
        }
    }

    /// 语法面 must 区分于背景（保证有语法着色时仍可读，而非隐形）。
    #[test]
    fn syntax_colors_are_distinct_from_background_in_both_themes() {
        for theme in [EditorTheme::dark(), EditorTheme::light()] {
            for kind in [
                "keyword",
                "string",
                "number",
                "comment",
                "type",
                "identifier",
                "field",
                "function",
                "attribute",
                "variable",
                "parameter",
                "boolean",
            ] {
                let c = theme.syntax_color(kind).expect("已知 kind 应有颜色");
                assert!(
                    contrast(c, theme.background) >= 2.0,
                    "{:?} 主题的 {kind} 色与背景对比度过低",
                    c,
                );
            }
        }
    }

    /// 未知语法 kind 返回 None，由调用方回退到默认文本色。
    #[test]
    fn syntax_color_ignores_unknown_kind() {
        for theme in [EditorTheme::dark(), EditorTheme::light()] {
            assert_eq!(theme.syntax_color("unknown_kind"), None);
            assert_eq!(theme.syntax_color("namespace"), None);
        }
    }

    /// 两套主题确实不同（避免误注入同一套导致明暗无效果）。
    #[test]
    fn dark_and_light_themes_differ() {
        let dark = EditorTheme::dark();
        let light = EditorTheme::light();
        assert_ne!(dark.background, light.background);
        assert_ne!(dark.text, light.text);
        assert_ne!(dark.completion_bg, light.completion_bg);
    }

    /// `Default` 兜底为暗黑主题（宿主尚未注入时行为明确）。
    #[test]
    fn default_falls_back_to_dark() {
        let theme = EditorTheme::default();
        assert_eq!(theme.background, EditorTheme::dark().background);
    }

    #[test]
    fn auto_close_pairs_cover_common_editor_delimiters() {
        assert_eq!(Editor::auto_close_pair('('), Some(')'));
        assert_eq!(Editor::auto_close_pair('['), Some(']'));
        assert_eq!(Editor::auto_close_pair('{'), Some('}'));
        assert_eq!(Editor::auto_close_pair('`'), Some('`'));
        assert_eq!(Editor::auto_close_pair('a'), None);
    }
}

// ---------------------------------------------------------------- 编辑器状态

pub(crate) struct Editor {
    /// 稳定的编辑器实例标识，供性能日志串联同一编辑器的异步任务。
    pub(crate) perf_editor_id: u64,
    /// 当前编辑事件标识，供同一次编辑启动的后台任务串联日志。
    perf_edit_id: u64,
    pub(crate) buffer: EditorBuffer,
    pub(crate) selection: Selection,
    pub(crate) providers: Providers,
    /// 创建时注入的通用能力配置，避免配置只停留在 core 协议层。
    pub(crate) profile: EditorProfile,
    pub(crate) font_name: String,
    pub(crate) gutter_line_numbers: bool,

    // 查找状态（通用编辑能力，宿主负责渲染 hosting 面板）
    pub(crate) find_state: FindState,
    pub(crate) find_input: gpui::Entity<InputState>,
    pub(crate) replace_input: gpui::Entity<InputState>,
    find_matches_cache: RefCell<Option<FindMatchesCache>>,

    // 视图状态
    pub(crate) focus_handle: FocusHandle,
    pub(crate) scroll_handle: ScrollHandle,
    pub(crate) cursor_visible: bool,
    #[allow(dead_code)] // 鼠标框选交互在 input.rs 实现，尚未接入宿主渲染。
    pub(crate) selecting_with_mouse: bool,
    pub(crate) ime_marked_range: Option<CoreRange>,
    pub(crate) soft_wrap: bool,
    /// 当前 viewport 对应的软换行宽度（UTF-16 列）。
    pub(crate) wrap_width_utf16: usize,
    pub(crate) font_size: f32,
    pub(crate) line_height: f32,
    #[allow(dead_code)] // 制表符宽度用于软换行列宽计算，当前固定按 4 列处理。
    pub(crate) tab_width: usize,
    /// 用户折叠的稳定状态（DM-106）：绑定到内容区间（AnchorRange）而非行号，
    /// 编辑后经 `relocate` 重定位；不再依赖行号集合猜测。
    pub(crate) folds: FoldSet,
    // 主题（整改设计 4.2）：由宿主依据 ThemeMode 注入，`Default` 暗黑为兜底。
    pub(crate) theme: EditorTheme,

    // 显示映射缓存（随编辑失效重建）
    pub(crate) display: DisplayMap,
    content_width_cache: RefCell<Option<ContentWidthCache>>,
    /// 单调增长的行宽上界；删除最长行时允许暂时高估，避免每次编辑全文扫描。
    line_width_hint: RefCell<Option<(usize, f32)>>,
    /// 当前 buffer/theme/font 下的可见行 shaping 缓存；编辑时只重定位未受影响行。
    shaped_line_cache: RefCell<HashMap<(u64, usize, usize, u32), ShapedLine>>,
    /// 同一 buffer 版本内复用 CodeLens 全集；视口变化只做本地过滤。
    code_lens_cache: RefCell<Option<(u64, usize, usize, Arc<Vec<CodeLens>>) >>,
    /// CodeLens 对 visual row 的映射只随 display map 变化。
    code_lens_visual_rows_cache: RefCell<Option<(u64, usize, Vec<usize>)>>,
    /// Inlay provider 结果按文档版本、provider revision 和 viewport 范围复用。
    inline_hint_cache: RefCell<Option<(u64, usize, usize, u64)>>,
    /// 语言折叠候选只随文档版本变化；滚动和重绘直接复用（字节区间, 行区间）。
    fold_candidates_cache: RefCell<Option<(u64, usize, usize, Vec<(CoreRange, Fold)>)>>,

    // 异步结果
    /// 最新一次语法解析结果（版本受保护，过期结果会被丢弃）。
    ///
    /// 渲染层据此按可见行读取 `syntax.highlights` 构建 token 着色的 TextRun。
    pub(crate) syntax: Option<SyntaxSnapshot>,
    /// 语法注入层树（DM-620/624）：由父 `SyntaxProvider::injections` 构建，供
    /// 按 offset 定位嵌套语言（hover/诊断/高亮路由）。单语言（无注入）时为空，
    /// 零开销。
    pub(crate) syntax_layers: SyntaxLayerTree,
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// 诊断按 buffer 行建立的下标，避免绘制每行扫描全文结果。
    pub(crate) diagnostic_line_index: Vec<Vec<usize>>,
    pub(crate) completion_items: Vec<CompletionItem>,
    pub(crate) completion_visible: bool,
    pub(crate) completion_loading: bool,
    /// 补全浮层锚点（字节偏移）：候选首次就绪时记录的光标位置。浮层打开后不随光标
    /// notify 重算几何，仅当光标移出该词替换区间时关闭——对齐 dbeaver(JFace) 的
    /// “锚定一次、离开即关”行为，避免“补全框跟着光标走”。
    pub(crate) completion_anchor_offset: Option<usize>,
    /// 续载游标（DM-704）：Some(游标) 表示 provider 返回不完整结果，下一次编辑须
    /// 重新请求 provider 续载；替代裸 `has_more: bool` 的能力上限。
    pub(crate) completion_continuation: Option<CompletionContinuation>,
    pub(crate) completion_selected: usize,
    /// 补全浮层原生滚动句柄：行列表由 GPUI `track_scroll` 容器驱动，滚轮/键盘
    /// 保持可见均走原生 ScrollHandle（对齐数据表格的原生滚动手感）。null 值仅分
    /// 布在浮层未打开时的虚拟字段语义之外。
    pub(crate) completion_scroll_handle: gpui::ScrollHandle,
    /// 补全浮层宽度（缓存）：候选首次就绪时对全量标签定型一次，避免滚动到不同
    /// 标签时逐帧重算宽度导致抖动与 render 内字形整形的开销。
    pub(crate) completion_width: Option<gpui::Pixels>,
    /// 当前补全查询词，供渲染层对 label 做命中高亮。
    pub(crate) completion_query: String,
    /// F005：选中项右侧 metadata 详情异步状态。选中/悬停切换触发，latest-wins 取消。
    pub(crate) completion_doc_state: Option<DocumentationState>,
    pub(crate) completion_ctrl: CompletionController,
    pub(crate) last_buffer_version_for_completion: u64,
    /// 接受补全写回文本时跳过一次自动补全，避免候选被立即重新弹出。
    suppress_next_auto_completion: bool,
    #[allow(dead_code)] // hover 悬浮提示由 input.rs 计算，尚未接入宿主渲染。
    pub(crate) hovered_point: Option<Point>,
    pub(crate) hover_content: Option<HoverContent>,
    #[allow(dead_code)] // 同上，hover 异步加载状态暂未消费。
    pub(crate) hover_loading: bool,

    // 函数签名提示（P1.9）
    /// 光标所在函数调用的签名提示；None 表示不显示。
    pub(crate) signature: Option<SignatureInfo>,

    // Snippet 会话（P3）：接受 snippet 候选后进入，Tab/Shift-Tab 切占位，Esc 退出。
    /// 活动 snippet 会话；None 表示无活动会话。
    pub(crate) snippet_session: Option<SnippetSession>,

    // 布局/命中
    pub(crate) line_hit_regions: Vec<LineHitRegion>,
    pub(crate) code_lens_hits: Vec<CodeLensHit>,
    #[allow(dead_code)] // 命中区域高亮状态暂未接入渲染。
    pub(crate) hovered_line_region: Option<usize>,
    pub(crate) gutter_hovered: bool,
    pub(crate) last_cursor_local_bounds: Option<gpui::Bounds<gpui::Pixels>>,

    // 滚动动画（整改 5.x）：滚轮只更新目标 offset；动画时钟以 `cx.on_next_frame`
    // 自接力按显示刷新率（~60Hz）把当前 offset 指数趋近目标，滚动帧不再锁死
    // 在滚轮事件节拍（实测 ~18Hz），实现平滑子像素滚动。目标是绝对位移（与
    // ScrollHandle offset 同坐标系，负 y = 向下），不引入过冲，故小滚动不会跳。
    scroll_target: gpui::Point<f32>,
    scroll_last_wheel_at: Option<Instant>,
    scroll_last_tick: Option<Instant>,
    scroll_animating: bool,
    /// 滚轮位移累积（跨 render 闭包保留）：同向滚动手势逐帧 `coalesce` 累加、
    /// 反向自动重置。由 render 根元素的 `on_scroll_wheel` 闭包更新，再交给
    /// `scroll` 做平滑动画。下沉前 SQL 面板用宿主侧 `Rc<Cell<ScrollDelta>>`
    /// 承担此职，现统一由编辑器自身持有（对齐 Zed 的增量滚动模型）。
    wheel_gesture_delta: Cell<ScrollDelta>,

    // 任务
    _completion_task: Option<Task<()>>,
    /// F005：详情异步解析任务；选中切换时旧任务被 token 判废（latest-wins）。
    _completion_doc_task: Option<Task<()>>,
    _hover_task: Option<Task<()>>,
    _diagnostics_task: Option<Task<()>>,
    _syntax_task: Option<Task<()>>,
    _signature_task: Option<Task<()>>,
    _cursor_blink_task: Option<Task<()>>,
    /// hover 取消令牌（DM-603）：新 hover 请求先 bump，旧任务在提交时连版本守卫
    /// 一起判废，保证陈旧 tooltip 不落地。
    _hover_token: CancellationToken,
    /// 补全 session 取消令牌（DM-703）：新请求先 `request_id()`，旧任务在提交时把
    /// `!session_token.check(request_id)` 叠进既有 version+is_latest_request 双守卫；
    /// hide/取消/接受路径 `cancel()` 令 in-flight 判废。
    _completion_session_token: CancellationToken,
    /// F005：详情解析取消令牌。选中/悬停切换先 bump，旧详情任务提交时判废，保证
    /// 旧结果不覆盖新选中项（latest-wins）。
    _completion_doc_token: CancellationToken,
    _subscriptions: Vec<Subscription>,

    // 仅用于性能采样：按编辑器实例每秒汇总一次实际 paint 耗时。
    perf_window_started: Instant,
    perf_frame_count: u32,
    perf_paint_total: Duration,
    perf_paint_max: Duration,
    perf_dropped_frames: u64,
    /// 最近一次编辑（after_edit）时刻。方案 A：编辑后的短暂窗口内强制连续排帧，
    /// 消除输入回显延迟；窗口过后回到事件驱动（静止无脏区不重绘）。
    last_edit_at: Instant,
}

/// 光标前一个词首（局部行扫描）。见 `Editor::prev_word_start`。
fn prev_word_start_in_snap(snap: &BufferSnapshot, cursor: usize) -> usize {
    let pos = cursor.min(snap.len());
    let mut row = snap.offset_to_point(pos).row;
    let mut seg_end = pos;
    loop {
        let line_start = snap.line_start(row);
        // 本行扫描窗口 [line_start, seg_end)，含换行（换行属于空白，复现原全量语义）。
        let seg = snap.text_in_range(CoreRange::new(line_start, seg_end));
        let b = seg.as_bytes();
        let mut i = b.len();
        while i > 0 && b[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        if i > 0 {
            while i > 0 && !b[i - 1].is_ascii_whitespace() {
                i -= 1;
            }
            return line_start + i;
        }
        // 本段全空白：向上跨一行。
        if row == 0 {
            return 0;
        }
        row -= 1;
        seg_end = snap.line_start(row + 1);
    }
}

/// 光标后下一个词首（局部行扫描）。两阶段：先跨过词（非空白），再跨过空白
/// （空白可跨越换行），返回之后第一个词首位置。
fn next_word_start_in_snap(snap: &BufferSnapshot, cursor: usize) -> usize {
    let pos = cursor.min(snap.len());
    let mut row = snap.offset_to_point(pos).row;
    let mut seg_start = pos;
    // 阶段 1：跳过非空白；阶段 2：跳过空白（可跨行）。
    let mut phase: u8 = 1;
    loop {
        let line_end = snap.line_end_offset(row);
        let seg = snap.text_in_range(CoreRange::new(seg_start, line_end));
        let b = seg.as_bytes();
        let mut i = 0usize;
        while i < b.len() {
            let is_ws = b[i].is_ascii_whitespace();
            if phase == 1 {
                if is_ws {
                    phase = 2;
                    continue;
                }
            } else if !is_ws {
                return seg_start + i;
            }
            i += 1;
        }
        // 本段耗尽：跨到下一行继续当前阶段。
        if row + 1 >= snap.line_count() {
            return snap.len();
        }
        seg_start = line_end;
        row += 1;
    }
}

#[cfg(test)]
mod range_index_tests {
    use super::*;

    #[test]
    fn indexes_ranges_by_every_covered_line() {
        let buffer = EditorBuffer::new_from("one\ntwo\nthree");
        let snapshot = buffer.snapshot();
        let index = fluxdb_editor_core::build_range_line_index(
            &snapshot,
            [CoreRange::new(1, 6), CoreRange::new(8, 8), CoreRange::new(100, 120)],
        );

        assert_eq!(index.len(), 3);
        assert_eq!(index[0], vec![0]);
        assert_eq!(index[1], vec![0]);
        assert!(index[2].is_empty());
    }
}

/// 活动 snippet 会话：记录插入后的占位范围，支持 Tab/Shift-Tab 切换与 Esc 退出。
///
/// 每个 tabstop 用绝对 buffer byte range 表示（相对于 snippet 插入点偏移），
/// 普通编辑导致占位范围失效时（越界或文本变化）自动结束会话。
#[derive(Clone, Debug)]
pub(crate) struct SnippetSession {
    /// 占位符列表（绝对 buffer byte range），按 tabstop 编号排序。
    pub(crate) tabstops: Vec<CoreRange>,
    /// 当前选中的占位下标。
    pub(crate) current: usize,
}

impl SnippetSession {
    /// 当前占位的绝对范围；无占位时返回 None。
    pub(crate) fn current_range(&self) -> Option<CoreRange> {
        self.tabstops.get(self.current).copied()
    }

    /// 切换到下一个占位（循环）；仅当多于一个占位时生效。
    pub(crate) fn next(&mut self) {
        if self.tabstops.len() > 1 {
            self.current = (self.current + 1) % self.tabstops.len();
        }
    }

    /// 切换到上一个占位（循环）。
    pub(crate) fn prev(&mut self) {
        if self.tabstops.len() > 1 {
            self.current = self
                .current
                .checked_sub(1)
                .unwrap_or(self.tabstops.len() - 1);
        }
    }

    /// 会话是否仍有效：所有占位范围落在 buffer 内且有序。
    ///
    /// 不校验 buffer 版本：在占位内编辑会推进版本，但占位范围仍有效，会话应保持。
    /// 仅当占位范围越界或乱序时判定失效（如全文替换、撤销后范围不再对应原文本）。
    pub(crate) fn is_valid(&self, buffer_len: usize) -> bool {
        self.tabstops
            .iter()
            .all(|r| r.start <= buffer_len && r.end <= buffer_len && r.start <= r.end)
    }
}

impl Editor {
    pub(crate) fn new(
        text: impl Into<String>,
        providers: Providers,
        config: Option<EditorConfig>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = text.into();
        let focus_handle = cx.focus_handle();
        // focus_handle 稍后会移入结构体，subscription 前先克隆一份。
        let focus_handle_sub = focus_handle.clone();
        let config = config.unwrap_or_default();
        let profile = config.profile.clone();
        let mut providers = providers;
        if let Some(registry) = &providers.language_registry {
            if let Some(entry) = registry.get(&profile.language_id) {
                if providers.language.is_none() {
                    providers.language = Some(entry.language);
                }
                if providers.syntax.is_none() {
                    providers.syntax = entry.syntax;
                }
            }
        }
        let soft_wrap = config.profile.soft_wrap == fluxdb_editor_core::SoftWrapMode::EditorWidth;
        let empty = EditorBuffer::new_from(&text);
        let perf_editor_id = NEXT_EDITOR_ID.fetch_add(1, Ordering::Relaxed);
        let find_input = cx.new(|cx| InputState::new(_window, cx).placeholder("查找"));
        let replace_input = cx.new(|cx| InputState::new(_window, cx).placeholder("替换"));
        let mut tool = Self {
            perf_editor_id,
            perf_edit_id: 0,
            buffer: empty, // 占位，下面重建
            selection: Selection::point(0),
            providers,
            profile,
            font_name: config.font.clone(),
            gutter_line_numbers: config.gutter_line_numbers,
            find_state: FindState::default(),
            find_input: find_input.clone(),
            replace_input: replace_input.clone(),
            find_matches_cache: RefCell::new(None),
            focus_handle,
            scroll_handle: ScrollHandle::new(),
            cursor_visible: false,
            selecting_with_mouse: false,
            ime_marked_range: None,
            soft_wrap,
            wrap_width_utf16: 120,
            font_size: if config.font_size > 0. {
                config.font_size
            } else {
                EDITOR_TEXT_SIZE
            },
            line_height: config.line_height.max(config.font_size + 2.),
            tab_width: config.profile.tab_size.max(1),
            folds: FoldSet::new(),
            theme: EditorTheme::default(),
            display: DisplayMap::new_with_tab_size(
                EditorBuffer::new_from(&text).snapshot(),
                SoftWrap::None,
                40,
                Vec::new(),
                config.profile.tab_size.max(1),
            ),
            content_width_cache: RefCell::new(None),
            line_width_hint: RefCell::new(None),
            shaped_line_cache: RefCell::new(HashMap::new()),
            code_lens_cache: RefCell::new(None),
            code_lens_visual_rows_cache: RefCell::new(None),
            inline_hint_cache: RefCell::new(None),
            fold_candidates_cache: RefCell::new(None),
            syntax: None,
            syntax_layers: SyntaxLayerTree::default(),
            diagnostics: Vec::new(),
            diagnostic_line_index: Vec::new(),
            completion_items: Vec::new(),
            completion_visible: false,
            completion_loading: false,
            completion_anchor_offset: None,
            completion_continuation: None,
            completion_selected: 0,
            completion_scroll_handle: gpui::ScrollHandle::new(),
            completion_width: None,
            completion_query: String::new(),
            completion_doc_state: None,
            completion_ctrl: CompletionController::new(),
            last_buffer_version_for_completion: 0,
            suppress_next_auto_completion: false,
            hovered_point: None,
            hover_content: None,
            hover_loading: false,
            signature: None,
            snippet_session: None,
            line_hit_regions: Vec::new(),
            code_lens_hits: Vec::new(),
            hovered_line_region: None,
            gutter_hovered: false,
            last_cursor_local_bounds: None,
            _completion_task: None,
            _completion_doc_task: None,
            _hover_task: None,
            _hover_token: CancellationToken::default(),
            _completion_doc_token: CancellationToken::default(),
            _completion_session_token: CancellationToken::default(),
            _diagnostics_task: None,
            _syntax_task: None,
            _signature_task: None,
            _cursor_blink_task: None,
            _subscriptions: Vec::new(),
            perf_window_started: Instant::now(),
            perf_frame_count: 0,
            perf_paint_total: Duration::ZERO,
            perf_paint_max: Duration::ZERO,
            perf_dropped_frames: 0,
            last_edit_at: Instant::now(),
            scroll_target: gpui::point(0.0, 0.0),
            scroll_last_wheel_at: None,
            scroll_last_tick: None,
            scroll_animating: false,
            wheel_gesture_delta: Cell::new(ScrollDelta::default()),
        };
        tool.buffer = EditorBuffer::new_from(&text);
        tool.rebuild_display();
        let focus_sub = cx.on_focus(&focus_handle_sub, _window, |this, _w, cx| {
            this.cursor_visible = true;
            this._cursor_blink_task = Some(cx.spawn(async move |this, cx| loop {
                smol::Timer::after(Duration::from_millis(500)).await;
                if this
                    .update(cx, |this, cx| {
                        this.cursor_visible = !this.cursor_visible;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }));
            cx.notify();
        });
        let blur_sub = cx.on_blur(&focus_handle_sub, _window, |this, _w, cx| {
            this.cursor_visible = false;
            this._cursor_blink_task = None;
            this.completion_visible = false;
            this.completion_scroll_handle.set_offset(gpui::Point::default());
            cx.notify();
        });
        tool._subscriptions = vec![focus_sub, blur_sub];
        tool._subscriptions.push(cx.subscribe(
            &find_input,
            |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.find_state.query = input.read(cx).value().to_string();
                    this.find_state.search_start = 0;
                    this.find_state.current_match = None;
                    this.find_state.found = false;
                    cx.notify();
                }
            },
        ));
        tool._subscriptions.push(cx.subscribe(
            &replace_input,
            |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.find_state.replace_text = input.read(cx).value().to_string();
                    cx.notify();
                }
            },
        ));
        if tool.providers.syntax.is_some() {
            let initial_change = TextChange::full_document(tool.buffer.to_string(), tool.buffer.version());
            tool.request_syntax(&initial_change, cx);
        }
        tool
    }

    // ------------------------------------------------------------ 公开 API（供接入层/宿主使用）

    pub(crate) fn text(&self) -> String {
        self.buffer.to_string()
    }

    pub(crate) fn text_in_range(&self, range: CoreRange) -> String {
        self.buffer.text_in_range(range)
    }

    pub(crate) fn selection_range(&self) -> CoreRange {
        self.selection.range()
    }

    /// 由宿主选择一个字节区间（例如 CodeLens 的 Select 动作）。
    pub(crate) fn select_range(&mut self, range: CoreRange, cx: &mut Context<Self>) {
        let start = range.start.min(self.buffer.len());
        let end = range.end.min(self.buffer.len());
        self.selection = Selection::new(start, end);
        cx.emit(EditorEvent::SelectionChanged(self.selection));
        cx.notify();
    }

    pub(crate) fn cursor_offset(&self) -> usize {
        self.selection.cursor.min(self.buffer.len())
    }

    pub(crate) fn perf_editor_id(&self) -> u64 {
        self.perf_editor_id
    }

    /// 选区文本（空选区返回 None）。供接入层（SQL 适配器）取选区以执行/补全。
    pub(crate) fn selected_text(&self) -> Option<String> {
        let r = self.selection_range();
        let start = r.start.min(self.buffer.len());
        let end = r.end.min(self.buffer.len());
        if start == end {
            return None;
        }
        Some(self.buffer.text_in_range(CoreRange::new(start, end)))
    }

    /// 当前是否软换行。
    pub(crate) fn soft_wrap(&self) -> bool {
        self.soft_wrap
    }

    /// 是否聚焦。
    #[allow(dead_code)] // 接入层公开 API，宿主暂未调用，保留供聚焦态判断使用。
    pub(crate) fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// 聚焦编辑器。
    #[allow(dead_code)] // 接入层公开 API，宿主暂用 focus_handle 直连，保留备用。
    pub(crate) fn focus(&mut self, _window: &mut Window) {
        self.cursor_visible = true;
    }

    /// 编辑器内缓冲区的总字节长度。
    #[allow(dead_code)] // 接入层公开 API，宿主暂未调用，保留备用。
    pub(crate) fn buffer_len(&self) -> usize {
        self.buffer.len()
    }

    /// 编辑器内缓冲区的行数。
    #[allow(dead_code)] // 接入层公开 API，宿主暂未调用，保留备用。
    pub(crate) fn line_count(&self) -> usize {
        self.buffer.line_count()
    }

    /// 静默同步外部文本（不触发 Changed 事件，且丢弃旧异步任务结果）。
    ///
    /// 只在外部模型「真正变化」时应用：先做 O(1) 长度比较，长度不同即视为变化，
    /// 避免为每次输入构造全文字符串（整改设计 4.1）。逐字输入走 `apply_edit` 的
    /// `Changed(TextChange)` 增量事件，宿主不会用本函数作为逐按键同步手段。
    pub(crate) fn sync_text_silent(&mut self, text: &str, cx: &mut Context<Self>) {
        // 长度不同 → 必然变化，直接应用；仅当长度相同才回退做一次全文比较（极少触发）。
        if self.buffer.len() == text.len() && self.buffer.to_string() == text {
            return;
        }
        self.cancel_async();
        self.content_width_cache.borrow_mut().take();
        self.line_width_hint.borrow_mut().take();
        self.fold_candidates_cache.borrow_mut().take();
        self.buffer = EditorBuffer::new_from(text);
        // 整篇替换文档：折叠锚点指向已失效内容，统一清空，避免解析到错误行（DM-106）。
        self.folds = FoldSet::new();
        self.selection = Selection::point(0);
        self.ime_marked_range = None;
        self.completion_visible = false;
        self.completion_loading = false;
        self.completion_continuation = None;
        self.completion_scroll_handle.set_offset(gpui::Point::default());
        self.completion_width = None;
        self.diagnostics.clear();
        self.diagnostic_line_index.clear();
        self.syntax = None;
        self.rebuild_display();
        if self.providers.syntax.is_some() {
            let change = TextChange::full_document(text.to_string(), self.buffer.version());
            self.request_syntax(&change, cx);
        }
        cx.notify();
    }

    #[allow(dead_code)] // 供宿主切换 provider 集合，暂未调用，保留。
    pub(crate) fn set_providers(&mut self, providers: Providers) {
        self.providers = providers;
        self.code_lens_cache.borrow_mut().take();
        self.code_lens_visual_rows_cache.borrow_mut().take();
        self.inline_hint_cache.borrow_mut().take();
        self.fold_candidates_cache.borrow_mut().take();
    }

    /// 由接入层/宿主调用，把当前 settings 应用到编辑器。
    /// `line_height` 为 0 时回退到 `font_size + 2.` 的默认行为。
    pub(crate) fn apply_settings(&mut self, font_size: f32, line_height: f32, soft_wrap: bool) {
        if font_size > 0. {
            if (self.font_size - font_size).abs() > f32::EPSILON {
                self.content_width_cache.borrow_mut().take();
                self.line_width_hint.borrow_mut().take();
                self.shaped_line_cache.borrow_mut().clear();
            }
            self.font_size = font_size;
            // 仅当调用方显式传入行高时才覆盖，避免字号调整吞掉用户自定义行高。
            if line_height > 0. {
                self.line_height = line_height;
            } else if (self.line_height - (font_size + 2.)).abs() <= f32::EPSILON {
                // 当前行高仍是默认派生值，随字号同步更新。
                self.line_height = font_size + 2.;
            }
        }
        if self.soft_wrap != soft_wrap {
            self.soft_wrap = soft_wrap;
            self.rebuild_display();
        }
    }

    /// 注入主题配色（整改设计 4.2）。主题只影响绘制颜色，不影响布局几何；
    /// 宿主在主题切换时对本编辑器调用此方法。
    pub(crate) fn set_theme(&mut self, theme: EditorTheme, cx: &mut Context<Self>) {
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        self.shaped_line_cache.borrow_mut().clear();
        cx.notify();
    }

    // ------------------------------------------------------------ 内部

    fn cancel_async(&mut self) {
        if let Some(completion) = self.providers.completion.clone() {
            completion.cancel_pending();
        }
        self._completion_task = None;
        self._hover_task = None;
        self._diagnostics_task = None;
        self._syntax_task = None;
        self._signature_task = None;
    }

    pub(crate) fn rebuild_display(&mut self) {
        let started = Instant::now();
        let soft = if self.soft_wrap { SoftWrap::EditorWidth } else { SoftWrap::None };
        // 折叠取稳定状态 `self.folds` 解析出的行区间，绝不构造 start == end 的
        // 退化折叠（见整改设计 5.4）。
        let folds = self.active_folds();
        self.display = DisplayMap::new_with_tab_size(
            self.buffer.snapshot(),
            soft,
            self.wrap_width_utf16(),
            folds,
            self.tab_width,
        );
        self.code_lens_visual_rows_cache.borrow_mut().take();
        self.sync_blocks();
        tracing::debug!(
            target: "gdb_editor_perf",
            op = "display_snapshot_commit",
            editor_id = self.perf_editor_id,
            edit_id = self.perf_edit_id,
            buffer_version = self.buffer.version(),
            request_id = 0u64,
            task_id = 0u64,
            layer = "display",
            elapsed_us = started.elapsed().as_micros() as u64,
            visual_rows = self.display.visual_row_count(),
            block_rows = self.display.block_total_rows(),
        );
    }

    /// 把 CodeLens 视觉行映射为 `Vec<Block>` 注入 DisplayMap 顶层 Block 层
    /// （DM-312）：滚动总高、scroll-to-cursor、hit test 统一经 Block 摘要查询，
    /// 删除原先散落的 `count(lens_row ≤ r)×CODE_LENS_HEIGHT` 旁路单调计数。
    fn sync_blocks(&mut self) {
        let started = Instant::now();
        let rows = self.code_lens_visual_rows();
        let block_count = rows.len();
        let blocks: Vec<Block> = rows
            .into_iter()
            .enumerate()
            .map(|(i, wrap_row)| Block {
                id: BlockId(i as u64 + 1),
                wrap_row,
                before: true,
                height_rows: 1,
                payload: i as u64,
            })
            .collect();
        // provider_revision 绑定 buffer version 与行数：任一变化即判过期重建。
        self.display
            .set_blocks(self.buffer.version(), blocks);
        tracing::debug!(
            target: "gdb_editor_perf",
            op = "block_update",
            editor_id = self.perf_editor_id,
            edit_id = self.perf_edit_id,
            buffer_version = self.buffer.version(),
            request_id = 0u64,
            task_id = 0u64,
            layer = "block",
            elapsed_us = started.elapsed().as_micros() as u64,
            block_count,
        );
    }

    fn rebuild_display_after_change(&mut self, change: &TextChange) {
        let soft = if self.soft_wrap { SoftWrap::EditorWidth } else { SoftWrap::None };
        // 先按本次编辑重定位稳定折叠（DM-106）：折叠绑定内容，编辑在前不漂移。
        self.folds.relocate(change.old_range, change.new_text.len());
        let folds = self.active_folds();
        if self.display.soft_wrap() == soft {
            let patch = self
                .display
                .apply_change_with_patch(self.buffer.snapshot(), change, folds);
            let (old_row_start, old_row_end, new_row_start, new_row_end) = patch
                .edits()
                .first()
                .map(|edit| {
                    (
                        edit.old_rows.start,
                        edit.old_rows.end,
                        edit.new_rows.start,
                        edit.new_rows.end,
                    )
                })
                .unwrap_or((0, 0, 0, 0));
            tracing::debug!(
                target: "gdb_editor_perf",
                op = "display_patch",
                editor_id = self.perf_editor_id,
                edit_id = self.perf_edit_id,
                buffer_version = self.buffer.version(),
                old_row_start,
                old_row_end,
                new_row_start,
                new_row_end,
                row_delta = patch.row_delta(),
            );
        } else {
            self.rebuild_display();
        }
        self.code_lens_visual_rows_cache.borrow_mut().take();
        self.sync_blocks();
    }

    /// 当前生效的折叠区间：把稳定折叠 `self.folds` 按当前快照解析回行区间 `Fold`。
    /// 没有用户折叠时不扫描 SQL 全文——这是逐字输入的热路径。
    fn active_folds(&self) -> Vec<Fold> {
        if self.folds.is_empty() {
            return Vec::new();
        }
        let snapshot = self.buffer.snapshot();
        self.folds.resolve_rows(|offset| {
            snapshot.offset_to_point(offset).row
        })
    }

    /// 语言 fold_ranges（字节区间）→ `(字节区间, 行区间 Fold)`，过滤退化的空/单行区间。
    /// 字节区间用于把「用户选择的折叠」存入稳定 `FoldSet`（DM-106），行区间供命中测试。
    fn fold_candidates(&self) -> Vec<(CoreRange, Fold)> {
        let Some(lang) = self.providers.language.clone() else {
            return Vec::new();
        };
        let snapshot = self.buffer.snapshot();
        let key = (snapshot.version(), snapshot.len(), snapshot.line_count());
        if let Some((version, bytes, lines, folds)) = self.fold_candidates_cache.borrow().as_ref()
            && (*version, *bytes, *lines) == key
        {
            return folds.clone();
        }
        let folds: Vec<(CoreRange, Fold)> = lang
            .fold_ranges(&snapshot)
            .into_iter()
            .filter_map(|r| {
                let start_row = snapshot.offset_to_point(r.start).row;
                // 区间终点是开区间，退格到该行内使 end 也指向所在行。
                let end_row = snapshot
                    .offset_to_point(r.end.saturating_sub(1))
                    .row;
                if end_row > start_row {
                    Some((r, Fold { start_row, end_row }))
                } else {
                    None
                }
            })
            .collect()
            ;
        let mut folds = folds;
        folds.sort_unstable_by_key(|(_, f)| f.start_row);
        *self.fold_candidates_cache.borrow_mut() = Some((key.0, key.1, key.2, folds.clone()));
        folds
    }

    /// 软换行宽度（UTF-16 列）。后续由布局根据视图宽度更新。
    pub(crate) fn wrap_width_utf16(&self) -> usize {
        self.wrap_width_utf16.max(1)
    }

    /// 根据当前滚动容器宽度更新软换行列数。
    pub(crate) fn update_wrap_width(&mut self, window: &Window) {
        if !self.soft_wrap {
            return;
        }
        let viewport_width = f32::from(self.scroll_handle.bounds().size.width);
        if viewport_width <= 1.0 {
            return;
        }
        let available = viewport_width
            - EDITOR_PADDING_X * 2.0
            - f32::from(self.line_number_width(window))
            - EDITOR_CONTENT_GAP;
        let width = (available / self.measure_character_width(window))
            .floor()
            .max(1.0) as usize;
        if width != self.wrap_width_utf16 {
            self.wrap_width_utf16 = width;
            self.rebuild_display();
        }
    }

    /// 在 prepaint 阶段把可见窗口的 inline hints 接入 DisplayMap。
    /// provider 结果绑定 buffer/provider revision 与范围，滚动或编辑只会刷新必要窗口。
    pub(crate) fn sync_inline_hints(&mut self, window: &Window) {
        let Some(provider) = self.providers.inline_hint.clone() else {
            return;
        };
        let snapshot = self.buffer.snapshot();
        let viewport = self.scroll_handle.bounds();
        let first = self.first_visible_visual_row(&viewport, self.scroll_handle.offset(), self.line_height(window));
        let overscan = (viewport.size.height / self.line_height(window)).ceil() as usize + 8;
        let last = first
            .saturating_add(overscan.max(1))
            .min(self.display.visual_row_count());
        let start = self
            .display
            .visual_line_at(first)
            .map(|line| self.buffer.line_start(line.buffer_row))
            .unwrap_or(0);
        let end = self
            .display
            .visual_line_at(last.saturating_sub(1))
            .map(|line| self.buffer.line_end_offset(line.buffer_row))
            .unwrap_or(start);
        let provider_revision = provider.revision();
        let key = (snapshot.version(), start, end, provider_revision);
        if self.inline_hint_cache.borrow().as_ref() == Some(&key) {
            return;
        }
        let hints = inline_hints_to_snapshot(
            snapshot.version(),
            provider.as_ref(),
            &snapshot,
            CoreRange::new(start, end),
        );
        let patch = self.display.set_inlays(hints);
        *self.inline_hint_cache.borrow_mut() = Some(key);
        if !patch.is_empty() {
            tracing::debug!(
                target: "gdb_editor_perf",
                op = "inlay_display_patch",
                editor_id = self.perf_editor_id,
                edit_id = self.perf_edit_id,
                buffer_version = snapshot.version(),
                provider_revision,
                old_rows = patch.old_row_bounds().map(|range| range.start),
                new_rows = patch.new_row_bounds().map(|range| range.start),
                row_delta = patch.row_delta(),
            );
        }
    }

    fn default_word_char(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }

    fn auto_close_pair(open: char) -> Option<char> {
        match open {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' => Some('"'),
            '\'' => Some('\''),
            '`' => Some('`'),
            _ => None,
        }
    }

    pub(crate) fn word_char(&self, c: char) -> bool {
        match &self.providers.language {
            Some(lang) => lang.word_char(c),
            None => Self::default_word_char(c),
        }
    }

    /// 处理一次编辑后的收尾：清理 IME 标记、重排语法/补全，并对外发布携带增量变更的
    /// `Changed(TextChange)` 事件，让宿主据此做增量文本同步（避免逐按键全文读取）。
    fn after_edit(&mut self, change: &TextChange, cx: &mut Context<Self>) {
        self.perf_edit_id = NEXT_EDIT_ID.fetch_add(1, Ordering::Relaxed);
        self.remap_shaped_line_cache(change);
        self.cursor_visible = true;
        self.ime_marked_range = None;
        // 方案 A：记录本次编辑时刻，paint 时据此在编辑后窗口内强制连续排帧（输入回显即时）。
        self.last_edit_at = Instant::now();
        // 保留未受编辑影响的高亮，避免防抖/后台解析期间整篇文本短暂退回默认色。
        // 编辑范围附近的 token 会被移除，待当前版本解析完成后再补齐。
        self.preserve_syntax_after_edit(change);
        self.preserve_diagnostics_after_edit(change);
        self.request_syntax(change, cx);
        self.request_diagnostics(change, cx);
        let suppress_completion = std::mem::take(&mut self.suppress_next_auto_completion);
        let query_char_count = self.completion_prefix().chars().count();
        let inserted_char_is_word = change
            .new_text
            .chars()
            .next()
            .is_some_and(|character| self.word_char(character));
        // 自动打开的门槛只管“新开浮层”；浮层已打开时，任何编辑（含删除）都必须重发
        // 请求以刷新候选，否则删除后仍显示删除前的旧候选。空查询/不足以触发时由
        // provider 在 should_trigger 阶段回绝并隐藏（见 request_completion 的 !proceed 分支）。
        let popup_open = self.completion_visible || self.completion_loading;
        let should_auto_complete = !suppress_completion
            && self.profile.should_auto_complete_after_edit(
                change,
                query_char_count,
                inserted_char_is_word,
            );
        if should_auto_complete || popup_open {
            self.request_completion(false, cx);
        } else if self._completion_task.is_some() {
            // 删除/粘贴/补全接受不会创建新请求，也必须让 provider 立即停止旧工作。
            if let Some(completion) = self.providers.completion.clone() {
                completion.cancel_pending();
            }
            self.completion_loading = false;
        }
        self.request_signature(cx);
        tracing::debug!(
            target: "gdb_editor",
            op = "after_edit",
            editor_id = self.perf_editor_id,
            edit_id = self.perf_edit_id,
            old_start = change.old_range.start,
            old_end = change.old_range.end,
            new_bytes = change.new_text.len(),
            cursor = self.selection.cursor,
            text_bytes = self.buffer.len(),
            line_count = self.buffer.line_count(),
            buffer_version = self.buffer.version(),
        );
        cx.emit(EditorEvent::Changed(change.clone()));
        cx.emit(EditorEvent::SelectionChanged(self.selection.clone()));
        cx.notify();
    }

    fn preserve_syntax_after_edit(&mut self, change: &TextChange) {
        let Some(previous) = self.syntax.take() else {
            return;
        };
        if change.full_document {
            // 全文档编辑：清空 store，等下一次全量结果 `from_highlights` 重建。
            self.syntax = Some(SyntaxSnapshot {
                buffer_version: change.version,
                highlights: previous.highlights,
                highlight_store: Arc::new(fluxdb_editor_core::HighlightStore::default()),
            });
            return;
        }
        // 逐字编辑：把 store 高亮的字节坐标按本次 delta 平移，保持与当前 buffer 版本一致；
        // 内容级替换待 async 结果到达时用 `apply_range` 收敛（DM-400/401）。
        let mut store = (*previous.highlight_store).clone();
        if !store.is_empty() {
            store.apply_edit(change.old_range, change.new_text.len(), self.buffer.len());
        }
        self.syntax = Some(SyntaxSnapshot {
            buffer_version: change.version,
            // Vec 仅作调试导出；渲染只走 HighlightStore（DM-406）。
            highlights: previous.highlights,
            highlight_store: Arc::new(store),
        });
    }

    /// 平移/保留未受编辑影响的诊断，避免每次按键整篇错误波浪线消失再回来（闪烁）。
    /// `full_document`（撤销/重做/set_text）才清空；普通编辑：编辑点前的诊断不变，
    /// 之后的按字节 delta 平移，与编辑区间相交的丢弃（由防抖后台解析重算补回）。
    fn preserve_diagnostics_after_edit(&mut self, change: &TextChange) {
        if change.full_document {
            self.diagnostics.clear();
            self.diagnostic_line_index.clear();
            return;
        }
        let old_start = change.old_range.start;
        let old_end = change.old_range.end;
        let delta = change.new_text.len() as isize - change.old_range.len() as isize;
        let shift = |offset: usize| {
            if delta >= 0 {
                offset.saturating_add(delta as usize)
            } else {
                offset.saturating_sub((-delta) as usize)
            }
        };
        if self.diagnostics.is_empty() {
            return;
        }
        let snapshot = self.buffer.snapshot();
        self.diagnostics = std::mem::take(&mut self.diagnostics)
            .into_iter()
            .filter_map(|mut diagnostic| {
                if diagnostic.range.end <= old_start {
                    // 编辑点之前：范围不变。
                    Some(diagnostic)
                } else if diagnostic.range.start >= old_end {
                    // 编辑点之后：整体平移 delta。
                    diagnostic.range.start = shift(diagnostic.range.start);
                    diagnostic.range.end = shift(diagnostic.range.end);
                    Some(diagnostic)
                } else {
                    // 与编辑区间相交：丢弃，防抖后重算补回。
                    None
                }
            })
            // 平移 delta 基于字节数，中文等多字节文档平移后可能落到字符中间，导致
            // 渲染按 TextRun 切片时 panic（gpui text_system）。渲染前吸附到字符边界。
            .filter_map(|mut diagnostic| {
                diagnostic.range.start =
                    snapshot.clamp_to_char_boundary(diagnostic.range.start);
                diagnostic.range.end = snapshot.clamp_to_char_boundary(diagnostic.range.end);
                (diagnostic.range.start < diagnostic.range.end).then_some(diagnostic)
            })
            .collect();
        self.diagnostic_line_index = fluxdb_editor_core::build_range_line_index(
            &snapshot,
            self.diagnostics.iter().map(|diagnostic| diagnostic.range),
        );
        // ponytail: 相交诊断直接丢弃、不做内容级修正；对语法错误标记足够，
        // 交给防抖重算收敛；如需更即时可再做内容智能，暂无必要。
    }

    /// 把 shaping 缓存中的未受影响行迁移到新 buffer 版本；dirty 行丢弃，编辑后的
    /// 后缀按字节 delta 平移。缓存上限很小，遍历它比重新 shape 整个 viewport 便宜。
    fn remap_shaped_line_cache(&mut self, change: &TextChange) {
        let old_version = change.version.saturating_sub(1);
        let old_start = change.old_range.start;
        let old_end = change.old_range.end;
        let delta = change.new_text.len() as isize - change.old_range.len() as isize;
        let mut cache = self.shaped_line_cache.borrow_mut();
        let entries: Vec<_> = cache.drain().collect();
        for ((version, start, end, font), shaped) in entries {
            if version != old_version {
                continue;
            }
            let key = if end <= old_start {
                (change.version, start, end, font)
            } else if start >= old_end {
                let shift = |offset: usize| {
                    if delta >= 0 {
                        offset.saturating_add(delta as usize)
                    } else {
                        offset.saturating_sub((-delta) as usize)
                    }
                };
                (change.version, shift(start), shift(end), font)
            } else {
                continue;
            };
            cache.insert(key, shaped);
        }
    }

    /// 用整文档变更驱动宿主同步（撤销/重做）。
    ///
    /// undo/redo 的变更范围不可预先精确追踪，故允许一次整文档读取换取正确性；
    /// 缩进/行注释/前缀插入等局部编辑已改走 `after_edit` 的增量变更（DM-503）。
    fn after_edit_full(&mut self, cx: &mut Context<Self>) {
        let change = TextChange::full_document(self.buffer.to_string(), self.buffer.version());
        self.after_edit(&change, cx);
    }

    // ------------------------------------------------------------ 编辑原语

    fn selection_start_end(&self) -> (usize, usize) {
        let r = self.selection.range();
        (
            r.start.min(self.buffer.len()),
            r.end.min(self.buffer.len()),
        )
    }

    /// 应用一次本地编辑；返回变化的旧区间。
    fn apply_edit(
        &mut self,
        new_text: &str,
        new_cursor: usize,
        new_anchor: usize,
        merge: bool,
        cx: &mut Context<Self>,
    ) -> Option<CoreRange> {
        let started = Instant::now();
        if self.profile.read_only {
            return None;
        }
        let old_range = self.selection.range();
        if old_range.is_empty() && new_text.is_empty() {
            return None;
        }
        let buffer_edit_started = Instant::now();
        let edit = self.buffer.edit_with_selection(
            old_range,
            new_text,
            new_cursor,
            new_anchor,
            self.selection.anchor,
            self.selection.cursor,
            merge,
        );
        self.update_line_width_hint(old_range.start, new_text);
        let buffer_edit_us = buffer_edit_started.elapsed().as_micros() as u64;
        self.selection = Selection::new(edit.anchor, edit.cursor);
        // 取本 edit 的增量变更广播给宿主；buffer 层保证至少返回一条。
        if let Some(change) = edit.changes.first() {
            let display_rebuild_started = Instant::now();
            self.rebuild_display_after_change(change);
            let display_rebuild_us = display_rebuild_started.elapsed().as_micros() as u64;
            let after_edit_started = Instant::now();
            self.after_edit(change, cx);
            let after_edit_us = after_edit_started.elapsed().as_micros() as u64;
            tracing::debug!(
                target: "gdb_editor_perf",
                op = "edit_to_event",
                elapsed_us = started.elapsed().as_micros() as u64,
                editor_id = self.perf_editor_id,
                edit_id = self.perf_edit_id,
                edit_bytes = new_text.len(),
                replaced_bytes = change.old_range.end.saturating_sub(change.old_range.start),
                text_bytes = self.buffer.len(),
                line_count = self.buffer.line_count(),
                buffer_version = self.buffer.version(),
                buffer_edit_us,
                display_rebuild_us,
                after_edit_us,
            );
            Some(change.old_range)
        } else {
            let display_rebuild_started = Instant::now();
            self.rebuild_display();
            let display_rebuild_us = display_rebuild_started.elapsed().as_micros() as u64;
            let after_edit_started = Instant::now();
            self.after_edit(&TextChange::new(old_range, new_text.to_string(), self.buffer.version()), cx);
            let after_edit_us = after_edit_started.elapsed().as_micros() as u64;
            tracing::debug!(
                target: "gdb_editor_perf",
                op = "edit_to_event",
                elapsed_us = started.elapsed().as_micros() as u64,
                editor_id = self.perf_editor_id,
                edit_id = self.perf_edit_id,
                edit_bytes = new_text.len(),
                replaced_bytes = old_range.end.saturating_sub(old_range.start),
                text_bytes = self.buffer.len(),
                line_count = self.buffer.line_count(),
                buffer_version = self.buffer.version(),
                buffer_edit_us,
                display_rebuild_us,
                after_edit_us,
            );
            Some(old_range)
        }
    }

    /// 在指定字节区间替换文本（供宿主侧参数替换 / 语句改写使用）。
    ///
    /// 与 `apply_edit` 不同，本方法不依赖当前选区，而是显式给定 `range` 作为
    /// 被替换区间；替换后光标折叠到替换结尾，写入撤销栈并触发 `EditorEvent::Changed`，
    /// 使宿主得以把新文本同步回查询模型。
    pub(crate) fn replace_text_range(
        &mut self,
        range: CoreRange,
        new_text: &str,
        cx: &mut Context<Self>,
    ) {
        if self.profile.read_only {
            return;
        }
        let new_cursor = range.start + new_text.len();
        let edit = self.buffer.edit(range, new_text, new_cursor, new_cursor, false);
        self.update_line_width_hint(range.start, new_text);
        self.selection = Selection::new(new_cursor, new_cursor);
        if let Some(change) = edit.changes.first() {
            self.rebuild_display_after_change(change);
            self.after_edit(change, cx);
        } else {
            self.rebuild_display();
        }
    }

    fn insert_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        if self.profile.auto_close_pairs
            && self.selection.is_empty()
            && text.chars().count() == 1
            && let Some(close) = text.chars().next().and_then(Self::auto_close_pair)
        {
            let mut paired = String::with_capacity(text.len() + close.len_utf8());
            paired.push_str(text);
            paired.push(close);
            self.apply_edit(&paired, cursor + text.len(), cursor + text.len(), true, cx);
            return;
        }
        let new_cursor = cursor + text.len();
        self.apply_edit(text, new_cursor, new_cursor, true, cx);
    }

    fn backspace(&mut self, cx: &mut Context<Self>) {
        let (start, end) = self.selection_start_end();
        if start != end {
            self.apply_edit("", start, start, true, cx);
            return;
        }
        if start == 0 {
            return;
        }
        // 用字符边界回退到前一个字符的起始，保证多字节字符整字删除、光标不落字符中间。
        // apply_edit 以「当前选区」作为被替换区间，故先把光标展开成 [前一字符, 光标] 选区，
        // 否则 old_range 为空区间，退格会变成空操作（与 delete 保持一致）。
        let prev = self.buffer.prev_char_boundary(start);
        self.selection = Selection::new(prev, start);
        self.apply_edit("", prev, prev, true, cx);
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        let (start, end) = self.selection_start_end();
        if start != end {
            self.apply_edit("", start, start, true, cx);
            return;
        }
        if start >= self.buffer.len() {
            return;
        }
        // 用完字符边界前进到该字符的结束偏移，删除其所在字符整体（旧实现误删 0 个字符）。
        let next = self.buffer.next_char_boundary(start);
        self.selection = Selection::new(start, next);
        self.apply_edit("", start, start, true, cx);
    }

    fn newline(&mut self, secondary: bool, cx: &mut Context<Self>) {
        if !self.profile.multiline {
            return;
        }
        let indent = self.compute_newline_indent();
        let text = if secondary {
            "\n".to_string()
        } else {
            format!("\n{indent}")
        };
        self.insert_text(&text, cx);
    }

    fn compute_newline_indent(&self) -> String {
        let cursor = self.cursor_offset();
        let point = self.buffer.offset_to_point(cursor);
        let line = self.buffer.line_text(point.row);
        line.chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect()
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        if self.profile.read_only {
            return;
        }
        let Some((anchor, cursor)) = self.buffer.undo() else {
            return;
        };
        self.selection = Selection::new(anchor, cursor);
        self.rebuild_display();
        // 撤销/重做不追踪精确区间，用整文档替换同步宿主；非逐按键路径。
        self.after_edit_full(cx);
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        if self.profile.read_only {
            return;
        }
        let Some((anchor, cursor)) = self.buffer.redo() else {
            return;
        };
        self.selection = Selection::new(anchor, cursor);
        self.rebuild_display();
        self.after_edit_full(cx);
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        let len = self.buffer.len();
        self.selection = Selection::new(0, len);
        cx.emit(EditorEvent::SelectionChanged(self.selection.clone()));
        cx.notify();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let (start, end) = self.selection_start_end();
        if start == end {
            return;
        }
        let text = self.buffer.text_in_range(CoreRange::new(start, end));
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
    }

    fn cut(&mut self, cx: &mut Context<Self>) {
        self.copy(cx);
        let (start, end) = self.selection_start_end();
        if start != end {
            self.apply_edit("", start, start, true, cx);
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let text = item.text().unwrap_or_default();
        self.insert_text(&text, cx);
    }

    // ------------------------------------------------------------ 缩进

    fn handle_indent(&mut self, outdent: bool, cx: &mut Context<Self>) {
        if self.profile.read_only {
            return;
        }
        let (start, end) = self.selection_start_end();
        if start != end {
            self.indent_selection(outdent, cx);
        } else if outdent {
            self.outdent_current_line(cx);
        } else {
            let unit = if self.profile.use_spaces {
                " ".repeat(self.tab_width)
            } else {
                "\t".to_string()
            };
            self.insert_text(&unit, cx);
        }
    }

    fn indent_selection(&mut self, outdent: bool, cx: &mut Context<Self>) {
        // 收集受影响行，自后往前应用行首缩进增删，避免偏移错位。
        let (start, end) = self.selection_start_end();
        let rows = self.indent_target_rows(start, end);
        let mut edits: Vec<(usize, usize, String)> = Vec::new(); // (line_start, 移除字节数, 插入文本)
        for row in rows {
            let line_start = self.buffer.line_start(row);
            let line_text = self.buffer.line_text(row);
            if outdent {
                let remove = line_text
                    .bytes()
                    .take(4)
                    .take_while(|b| *b == b' ' || *b == b'\t')
                    .count();
                edits.push((line_start, remove, String::new()));
            } else {
                edits.push((line_start, 0, "    ".to_string()));
            }
        }
        if edits.is_empty() {
            return;
        }
        let cursor = self.cursor_offset();
        // 收集本次缩进产生的增量变更；最后落盘的那条带最终 buffer 版本。
        // 多行缩进一个 TextChange 只精确承载一个 range，其余行高亮由后台 syntax
        // 结果（dirty_ranges）收敛，不再整文档 `to_string()`（DM-503）。
        let mut last_change: Option<TextChange> = None;
        for (pos, remove, insert) in edits.into_iter().rev() {
            let edit = if insert.is_empty() {
                self.buffer.edit(
                    CoreRange::new(pos, pos + remove),
                    "",
                    cursor,
                    cursor,
                    false,
                )
            } else {
                self.buffer.edit(
                    CoreRange::new(pos, pos),
                    &insert,
                    cursor,
                    cursor,
                    false,
                )
            };
            last_change = edit.changes.first().cloned();
        }
        self.rebuild_display();
        if let Some(change) = last_change {
            self.after_edit(&change, cx);
        }
    }

    fn outdent_current_line(&mut self, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let point = self.buffer.offset_to_point(cursor);
        let line_start = self.buffer.line_start(point.row);
        let line_text = self.buffer.line_text(point.row);
        let remove = line_text
            .bytes()
            .take(4)
            .take_while(|b| *b == b' ' || *b == b'\t')
            .count();
        if remove == 0 {
            return;
        }
        let new_cursor = self.cursor_offset().saturating_sub(remove);
        let edit = self.buffer.edit(
            CoreRange::new(line_start, line_start + remove),
            "",
            new_cursor,
            new_cursor,
            false,
        );
        self.selection = Selection::point(new_cursor);
        self.rebuild_display();
        // 局部单行编辑，用增量变更驱动宿主同步，避免整文档 `to_string()`（DM-503）。
        if let Some(change) = edit.changes.first() {
            self.after_edit(change, cx);
        }
    }

    fn indent_target_rows(&self, start: usize, end: usize) -> Vec<usize> {
        let start_row = self.buffer.offset_to_point(start).row;
        let end_point = self.buffer.offset_to_point(end);
        let end_row = if self.buffer.line_start(end_point.row) == end {
            end_point.row.saturating_sub(1)
        } else {
            end_point.row
        };
        (start_row..=end_row.min(self.buffer.line_count().saturating_sub(1))).collect()
    }

    // ------------------------------------------------------------ 光标移动

    fn move_for_action(&mut self, go: CursorMove, select: bool, cx: &mut Context<Self>) {
        self.cursor_visible = true;
        let cursor = self.cursor_offset();
        let point = self.buffer.offset_to_point(cursor);
        let new_cursor = match go {
            CursorMove::Left => self.buffer.prev_char_boundary(cursor),
            CursorMove::Right => self.buffer.next_char_boundary(cursor),
            CursorMove::Up => self.move_vertically(point, -1),
            CursorMove::Down => self.move_vertically(point, 1),
            CursorMove::Home => self.buffer.line_start(point.row),
            CursorMove::End => self.buffer.line_end_offset(point.row),
            CursorMove::Start => 0,
            CursorMove::EndAll => self.buffer.len(),
            CursorMove::PrevWord => self.prev_word_at(cursor),
            CursorMove::NextWord => self.next_word_at(cursor),
        };
        let new_cursor = new_cursor.min(self.buffer.len());
        // 锚定浮层：光标移出锚点词替换区间即关闭（对齐 dbeaver/JFace：离开即关）。
        if self.completion_visible && !self.cursor_within_completion_anchor(new_cursor) {
            self.completion_visible = false;
            self.completion_loading = false;
            self.completion_anchor_offset = None;
            // F005：关闭同时判废详情任务并清空右侧详情。
            self._completion_doc_token.cancel();
            self._completion_doc_task = None;
            self.completion_doc_state = None;
        }
        let anchor = if select {
            self.selection.anchor.min(self.buffer.len())
        } else {
            new_cursor
        };
        self.selection = Selection::new(anchor, new_cursor);
        self.ensure_cursor_visible();
        self.request_signature(cx);
        cx.emit(EditorEvent::SelectionChanged(self.selection.clone()));
        cx.notify();
    }

    fn move_vertically(&self, point: Point, delta: isize) -> usize {
        let current_col = self.buffer.utf16_column_at(point);
        let row_count = self.buffer.line_count();
        if row_count == 0 {
            return 0;
        }
        let target_row = if delta < 0 {
            point.row.saturating_sub(1)
        } else {
            (point.row as isize + delta).min(row_count as isize - 1) as usize
        };
        self.row_col_to_offset(target_row, current_col)
    }

    fn row_col_to_offset(&self, row: usize, utf16_col: usize) -> usize {
        let row = row.min(self.buffer.line_count().saturating_sub(1));
        let line = self.buffer.line_text(row);
        let mut cur = 0usize;
        let mut byte_col = line.len();
        for (i, ch) in line.char_indices() {
            let w = ch.len_utf16();
            if cur + w > utf16_col {
                byte_col = i;
                break;
            }
            cur += w;
            byte_col = i + ch.len_utf8();
        }
        self.buffer.line_start(row) + byte_col.min(line.len())
    }

    /// 光标前一个词首。逐行向上扫描，只读取光标附近若干行，不构造全文字符串
    /// （整改设计 4.1：删除键盘词移动的全文 `buffer.to_string()`）。
    fn prev_word_at(&self, cursor: usize) -> usize {
        prev_word_start_in_snap(&self.buffer.snapshot(), cursor)
    }

    /// 光标后下一个词首。逐行向下扫描，只读取光标附近若干行，不构造全文字符串
    /// （整改设计 4.1）。
    fn next_word_at(&self, cursor: usize) -> usize {
        next_word_start_in_snap(&self.buffer.snapshot(), cursor)
    }

    fn ensure_cursor_visible(&self) {
        let viewport = self.scroll_handle.bounds();
        let viewport_width = f32::from(viewport.size.width);
        let viewport_height = f32::from(viewport.size.height);
        if viewport_width <= 0.0 || viewport_height <= 0.0 {
            return;
        }
        let point = self.buffer.offset_to_point(self.cursor_offset());
        let visual_row = self
            .display
            .visual_row_for_column(point.row, point.column) as f32;
        let stride = self.line_height.max(self.font_size + 1.0) + EDITOR_LINE_GAP;
        let cursor_top = EDITOR_PADDING_Y + visual_row * stride;
        let cursor_bottom = cursor_top + self.line_height.max(self.font_size + 1.0);
        let scroll = self.scroll_handle.offset();
        let visible_top = -f32::from(scroll.y);
        let visible_bottom = visible_top + viewport_height;

        // 水平跟焦：光标本地 x（相对视口内容区左缘），用「半角字符宽 ≈ 0.5em + 边距」估算即可，
        // 仅用于判断光标是否超出视口左右，无需逐字 shape。出界时把视口滚到光标处并留一个字符
        // 边距，避免光标跑出视口不可见（对齐 zed autoscroll_horizontally / vscode reveal position）。
        let char_width = self.font_size * 0.5;
        let gutter_width = if self.gutter_line_numbers || self.profile.show_line_numbers {
            (self.line_digits() as f32 * 8.0 + EDITOR_GUTTER_GAP + EDITOR_FOLD_GUTTER)
                .max(EDITOR_MIN_GUTTER)
        } else {
            0.0
        };
        // 用 UTF-16 列估算光标 x（与布局列宽口径一致）。
        let column_utf16 = self.buffer.utf16_column_at(point);
        let cursor_local_x =
            EDITOR_PADDING_X + gutter_width + EDITOR_CONTENT_GAP + column_utf16 as f32 * char_width;
        let margin = char_width;
        let target_x = if cursor_local_x < margin {
            cursor_local_x - margin
        } else if cursor_local_x > viewport_width - margin {
            -(cursor_local_x - (viewport_width - margin))
        } else {
            f32::from(scroll.x)
        };

        let target_y = if cursor_top < visible_top {
            -cursor_top
        } else if cursor_bottom > visible_bottom {
            -(cursor_bottom - viewport_height)
        } else {
            f32::from(scroll.y)
        };
        let max_x = f32::from(self.scroll_handle.max_offset().x);
        let max_y = f32::from(self.scroll_handle.max_offset().y);
        let set_x = if (target_x - f32::from(scroll.x)).abs() > 0.5 {
            target_x.clamp(-max_x, 0.0)
        } else {
            f32::from(scroll.x)
        };
        let set_y = if (target_y - f32::from(scroll.y)).abs() > 0.5 {
            target_y.clamp(-max_y, 0.0)
        } else {
            f32::from(scroll.y)
        };
        self.scroll_handle
            .set_offset(gpui::point(px(set_x), px(set_y)));
    }

    // ------------------------------------------------------------ 折叠

    fn toggle_fold_at_cursor(&mut self, cx: &mut Context<Self>) {
        if !self.profile.show_folding {
            return;
        }
        let cursor = self.cursor_offset();
        let row = self.buffer.offset_to_point(cursor).row;
        self.toggle_fold_row(row, cx);
    }

    fn toggle_fold_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let candidates = self.fold_candidates();
        // 该行是某折叠起始行 → 切换该折叠的启停（以稳定字节区间存入 FoldSet）。
        if let Some((range, _)) = candidates.iter().find(|(_, f)| f.start_row == row) {
            self.folds.toggle_range(*range);
        } else if let Some((range, _)) = candidates
            .iter()
            .find(|(_, f)| row > f.start_row && row <= f.end_row)
        {
            // 点击折叠内部行 → 展开其所在折叠（只移除，绝不新增折叠）。
            self.folds.remove(*range);
        }
        self.rebuild_display();
        cx.notify();
    }

    fn fold_all(&mut self, cx: &mut Context<Self>) {
        if !self.profile.show_folding {
            return;
        }
        // 折叠所有语言折叠区间（非退化），而不是把每行当折叠区间。
        let ranges = self.fold_candidates().into_iter().map(|(r, _)| r);
        self.folds = FoldSet::from_byte_ranges(ranges);
        self.rebuild_display();
        cx.notify();
    }

    fn unfold_all(&mut self, cx: &mut Context<Self>) {
        if !self.profile.show_folding {
            return;
        }
        self.folds = FoldSet::new();
        self.rebuild_display();
        cx.notify();
    }

    // ------------------------------------------------------------ 补全

    fn completion_prefix(&self) -> String {
        let snapshot = self.buffer.snapshot();
        let cursor = self.cursor_offset();
        fluxdb_editor_core::completion_prefix(&snapshot, cursor, &|c| self.word_char(c))
    }

    fn request_completion(&mut self, explicit: bool, cx: &mut Context<Self>) {
        let Some(completion) = self.providers.completion.clone() else {
            return;
        };
        let editor_id = self.perf_editor_id;
        let edit_id = self.perf_edit_id;
        let cursor = self.cursor_offset();
        let query = self.completion_prefix();
        let version = self.buffer.snapshot().version();
        tracing::debug!(
            target: "gdb_editor_perf",
            op = "completion_trigger",
            editor_id,
            edit_id,
            buffer_version = version,
            session = self.session_state().as_str(),
            query_len = query.len(),
            explicit,
        );

        // 触发决策委托给 provider：让 SQL 层能依据「标识符.」（SELECT u.）等在空前缀下也触发。
        // 决策用临时请求（request_id 不参与），仅当放行时才 new_request 抢占最新请求号。
        let decision = {
            let snapshot = self.buffer.snapshot();
            let decision_request = CompletionRequest {
                request_id: 0,
                buffer_version: version,
                cursor,
                query: query.clone(),
                explicit,
                document: Some(snapshot),
                edit_id,
                continuation: None,
            };
            completion.should_trigger(&decision_request)
        };
        let proceed = match decision {
            TriggerDecision::Yes => true,
            TriggerDecision::DependsOnPrefix(min) => {
                query.chars().count() >= min || explicit
            }
            TriggerDecision::No => false,
        };
        if !proceed {
            completion.cancel_pending();
            // DM-703：拒绝触发即 bump session，令 in-flight 旧任务判废。
            self._completion_session_token.cancel();
            tracing::debug!(
                target: "gdb_editor_perf",
                op = "completion_cancel",
                    task_outcome = TaskOutcome::CancelRequested.as_str(),
                editor_id,
                edit_id,
                buffer_version = version,
                session = self.session_state().as_str(),
                reason = "trigger_rejected",
                explicit,
            );
            self.completion_visible = false;
            self.completion_loading = false;
            self.completion_continuation = None;
            self.completion_scroll_handle.set_offset(gpui::Point::default());
            self.completion_width = None;
            self.completion_anchor_offset = None;
            return;
        }

        // 本地后缀复用：文本版本未变且新查询是上次查询的后缀。
        if self.completion_continuation.is_none() {
            if let Some(reused) = self
            .completion_ctrl
            .try_reuse_local_at(
                &query,
                version,
                self.last_buffer_version_for_completion,
                Some(cursor),
            )
            {
            self.completion_items = self.filter_sort_completion(reused, &query);
            tracing::debug!(
                target: "gdb_editor_perf",
                op = "completion_reuse",
                editor_id,
                edit_id,
                buffer_version = version,
                session = self.session_state().as_str(),
                query_len = query.len(),
                item_count = self.completion_items.len(),
            );
            self.completion_query = query.clone();
            self.completion_selected = 0;
            self.completion_visible = !self.completion_items.is_empty();
            self.completion_loading = false;
            self.completion_continuation = None;
            self.completion_scroll_handle.set_offset(gpui::Point::default());
            self.completion_scroll_handle.scroll_to_item(0);
            self.completion_width = None;
            self.capture_completion_anchor(cursor);
            self.request_completion_documentation(cx);
            cx.notify();
            return;
            }
        }

        let request = self
            .completion_ctrl
            .new_request(version, cursor, query.clone(), explicit);
        let mut request = request;
        request.edit_id = edit_id;
        // 仅把快照交给 provider；宿主不再为补全重新读取实体全文。
        request.document = Some(self.buffer.snapshot());
        let request_id = request.request_id;
        // DM-703：本次 session 的取消 id；与既有 version + is_latest_request 合成三守卫，
        // 旧任务在提交时对不上最新 session_id 即判废（hide/接受/新一轮请求都 bump）。
        let session_id = self._completion_session_token.request_id();
        let session_token = self._completion_session_token.clone();
        self.last_buffer_version_for_completion = version;
        self.completion_loading = true;
        self.completion_continuation = None;
        let provider_started = Instant::now();
        let future = completion.complete(request);
        tracing::debug!(
            target: "gdb_editor_perf",
            op = "completion_prepare",
            elapsed_us = provider_started.elapsed().as_micros() as u64,
            editor_id,
            edit_id,
            request_id,
            buffer_version = version,
            session = self.session_state().as_str(),
            query_len = query.len(),
            text_bytes = self.buffer.len(),
            explicit,
        );
        // 新请求会覆盖旧任务句柄（旧 future 与编辑器解绑继续运行但结果被丢弃）。
        // GPUI Task 无法真正中途取消，因此“丢弃旧结果”由下方双保护守卫保证：
        // 新请求推进 latest_request_id + 文本版本比对，二者任一不符即拒绝迟到结果。
        if self._completion_task.is_some() {
            tracing::debug!(
                target: "gdb_editor_perf",
                op = "completion_retrigger",
                editor_id,
                edit_id,
                request_id,
                buffer_version = version,
                session = self.session_state().as_str(),
            );
        }
        self._completion_task = None;
        let task_id = NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed);
        let task = cx.spawn(async move |this, cx| {
            // provider 可能包含昂贵的 schema 过滤；放到后台 executor，避免阻塞 UI 帧循环。
            let task_started = Instant::now();
            // 连续输入时只让最后一次请求进入 provider，避免每个字符都触发 metadata 扫描。
            if !explicit {
                smol::Timer::after(Duration::from_millis(PROVIDER_DEBOUNCE_MS)).await;
            }
            let current = this
                .update(cx, |this, _| this.buffer.version() == version)
                .unwrap_or(false);
            if !current {
                tracing::debug!(
                    target: "gdb_editor_perf",
                    op = "completion_result_discarded",
                        task_outcome = TaskOutcome::ResultDiscarded.as_str(),
                    phase = "debounce",
                    elapsed_us = task_started.elapsed().as_micros() as u64,
                    editor_id,
                    edit_id,
                    task_id,
                    request_id,
                    buffer_version = version,
                );
                return;
            }
            let result = cx.background_spawn(future).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(resp) => {
                        let item_count = resp.items.len();
                        // 三守卫（DM-703）：最新请求 + 文本版本未变 + session 未过期才采纳。
                        if this.buffer.snapshot().version() != version
                            || !this.completion_ctrl_accept_request(request_id)
                            || !session_token.check(session_id)
                        {
                            this.completion_loading = false;
                            tracing::debug!(
                                target: "gdb_editor_perf",
                                op = "completion_result_discarded",
                                    task_outcome = TaskOutcome::ResultDiscarded.as_str(),
                                elapsed_us = task_started.elapsed().as_micros() as u64,
                                editor_id,
                                edit_id,
                                task_id,
                                request_id,
                                buffer_version = version,
                                session = this.session_state().as_str(),
                                item_count,
                            );
                            return;
                        }
                        this.completion_ctrl.store_result_at(
                            &query,
                            resp.items.clone(),
                            Some(version),
                            Some(cursor),
                        );
                        this.completion_items = this.filter_sort_completion(resp.items, &query);
                        this.completion_continuation = resp.continuation;
                        this.completion_query = query.clone();
                        this.completion_selected = 0;
                        this.completion_visible = !this.completion_items.is_empty();
                        this.completion_loading = false;
                        this.completion_scroll_handle.set_offset(gpui::Point::default());
                        this.completion_scroll_handle.scroll_to_item(0);
                        this.completion_width = None;
                        this.capture_completion_anchor(cursor);
                        this.request_completion_documentation(cx);
                        cx.notify();
                        tracing::debug!(
                            target: "gdb_editor_perf",
                            op = "completion_result",
                            elapsed_us = task_started.elapsed().as_micros() as u64,
                            editor_id,
                            edit_id,
                            task_id,
                            request_id,
                            buffer_version = version,
                            session = this.session_state().as_str(),
                            item_count,
                            accepted_count = this.completion_items.len(),
                        );
                    }
                    Err(error) => {
                        this.completion_loading = false;
                        tracing::debug!(
                            target: "gdb_editor_perf",
                            op = "completion_error",
                            elapsed_us = task_started.elapsed().as_micros() as u64,
                            editor_id,
                            edit_id,
                            task_id,
                            request_id,
                            buffer_version = version,
                            session = this.session_state().as_str(),
                            error = ?error,
                        );
                    }
                }
            });
            // 任务签名与旧 sql_editor 一致（Task<()>），不再返回 Result。
        });
        self._completion_task = Some(task);
    }

    /// 刷新函数签名提示（P1.9）：按 Zed 的方式防抖并放到后台计算，避免编辑主线程
    /// 为大文档构造全文字符串和扫描括号；结果只接受当前 buffer version。
    fn request_signature(&mut self, cx: &mut Context<Self>) {
        let Some(signature) = self.providers.signature.clone() else {
            self.signature = None;
            return;
        };
        self.signature = None;
        let cursor = self.cursor_offset();
        let snapshot = self.buffer.snapshot();
        let version = snapshot.version();
        let position = snapshot.offset_to_point(cursor);
        let editor_id = self.perf_editor_id;
        let edit_id = self.perf_edit_id;
        let task_id = NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed);
        let task = cx.spawn(async move |this, cx| {
            let started = Instant::now();
            smol::Timer::after(Duration::from_millis(PROVIDER_DEBOUNCE_MS)).await;
            let result = cx
                .background_spawn(async move { signature.signature(&snapshot, position) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.buffer.snapshot().version() != version {
                    tracing::debug!(
                        target: "gdb_editor_perf",
                        op = "signature_discarded",
                            task_outcome = TaskOutcome::ResultDiscarded.as_str(),
                        editor_id,
                        edit_id,
                        task_id,
                        buffer_version = version,
                        request_id = 0u64,
                        layer = "signature",
                        elapsed_us = started.elapsed().as_micros() as u64,
                    );
                    return;
                }
                this.signature = result;
                cx.notify();
                tracing::debug!(
                    target: "gdb_editor_perf",
                    op = "signature_result",
                    task_outcome = TaskOutcome::ResultCommitted.as_str(),
                    editor_id,
                    edit_id,
                    task_id,
                    buffer_version = version,
                    request_id = 0u64,
                    layer = "signature",
                    elapsed_us = started.elapsed().as_micros() as u64,
                );
            });
        });
        self._signature_task = Some(task);
    }

    /// 补全控制器：仅当 request_id 仍是最新请求时返回 true（供任务回调检查）。
    ///
    /// 每次 `new_request` 都会推进 `latest_request_id`，因此即使两个请求处于同一 buffer
    /// version（如显式重复触发），旧请求的迟到结果也会在这里被正确拒绝，不会覆盖最新状态。
    fn completion_ctrl_accept_request(&self, request_id: u64) -> bool {
        self.completion_ctrl.is_latest_request(request_id)
    }

    fn filter_sort_completion(&self, items: Vec<CompletionItem>, query: &str) -> Vec<CompletionItem> {
        filter_completion_items_preserving_order(items, query)
    }

    #[allow(dead_code)] // 宿主驱动补全的入口，当前 SQL 适配器内部直接请求，暂未调用，保留。
    pub(crate) fn begin_completion_request(
        &mut self,
        explicit: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(u64, usize, String)> {
        let cursor = self.cursor_offset();
        let query = self.completion_prefix();
        let version = self.buffer.snapshot().version();
        let request = self
            .completion_ctrl
            .new_request(version, cursor, query.clone(), explicit);
        // 由宿主驱动的补全（复用外部 fluxdb-app 补全）时，不再次 async 请求。
        // 这里仅分配请求号并返回给宿主。
        let _ = cx;
        Some((request.request_id, cursor, query))
    }

    /// 宿主把外部补全结果喂回编辑器。
    #[allow(dead_code)] // 与 begin_completion_request 配套的宿主驱动补全入口，暂未调用，保留。
    pub(crate) fn inject_completion_results(
        &mut self,
        items: Vec<CompletionItem>,
        query: &str,
        cx: &mut Context<Self>,
    ) {
        self.completion_ctrl.store_result_at(
            query,
            items.clone(),
            Some(self.buffer.version()),
            Some(self.cursor_offset()),
        );
        self.completion_items = self.filter_sort_completion(items, query);
        self.completion_query = query.to_string();
        self.completion_selected = 0;
        self.completion_visible = !self.completion_items.is_empty();
        self.completion_loading = false;
        self.completion_continuation = None;
        self.completion_scroll_handle.set_offset(gpui::Point::default());
        self.completion_scroll_handle.scroll_to_item(0);
        self.completion_width = None;
        self.capture_completion_anchor(self.cursor_offset());
        self.request_completion_documentation(cx);
        cx.notify();
    }

    pub(crate) fn accept_completion(&mut self, item: CompletionItem, cx: &mut Context<Self>) {
        tracing::debug!(
            target: "gdb_editor_perf",
            op = "completion_accept",
            session = self.session_state().as_str(),
        );
        let cursor = self.cursor_offset();
        let prefix = self.completion_prefix();
        let fallback_start = cursor.saturating_sub(prefix.len());
        let requested = item
            .replace_range
            .unwrap_or_else(|| CoreRange::new(fallback_start, cursor));
        let replace_start = requested.start.min(self.buffer.len());
        let replace_end = requested.end.min(self.buffer.len()).max(replace_start);
        let replace_start = replace_start.min(self.buffer.len());
        let replace_end = replace_end.min(self.buffer.len()).max(replace_start);
        // Snippet 解析：仅显式声明 Snippet 格式时解析 tabstop，否则原样插入。
        let (insert_text, tabstops) = match item.insert_text_format {
            InsertTextFormat::Snippet => match fluxdb_editor_core::parse_snippet(&item.insert_text) {
                Ok(snippet) => (snippet.text, snippet.tabstops),
                Err(_) => {
                    // 解析失败回退纯文本，不把控制标记插入用户文本。
                    (item.insert_text.clone(), Vec::new())
                }
            },
            InsertTextFormat::PlainText => (item.insert_text.clone(), Vec::new()),
        };
        let new_cursor = replace_start + insert_text.len();
        self.selection = Selection::new(replace_start, replace_end);
        self.suppress_next_auto_completion = true;
        if self
            .apply_edit(&insert_text, new_cursor, new_cursor, true, cx)
            .is_none()
        {
            self.suppress_next_auto_completion = false;
        }
        // 有占位符时进入 snippet 会话：选中首个占位，Tab/Shift-Tab 切换。
        if !tabstops.is_empty() {
            let abs_tabstops: Vec<CoreRange> = tabstops
                .iter()
                .map(|r| CoreRange::new(replace_start + r.start, replace_start + r.end))
                .collect();
            self.snippet_session = Some(SnippetSession {
                tabstops: abs_tabstops.clone(),
                current: 0,
            });
            // 选中首个占位范围。
            if let Some(first) = abs_tabstops.first() {
                self.selection = Selection::new(first.start, first.end);
            }
        }
        self.completion_visible = false;
        self.completion_scroll_handle.set_offset(gpui::Point::default());
        self.completion_width = None;
        self.completion_anchor_offset = None;
        // DM-703：接受即 bump session，令仍 in-flight 的请求在提交时判废。
        self._completion_session_token.cancel();
        cx.emit(EditorEvent::CompletionAccepted(item));
    }

    /// 补全列表当前是否可见。
    pub(crate) fn completion_visible(&self) -> bool {
        self.completion_visible
    }

    /// 候选首次就绪时记录锚点：仅当浮层可用（visible 或 loading）时记录，且只在
    /// 锚点尚未设置时更新，避免续载/重触发把我们已在跟踪的锚点悄悄挪走。
    fn capture_completion_anchor(&mut self, cursor: usize) {
        if (self.completion_visible || self.completion_loading) && self.completion_anchor_offset.is_none() {
            self.completion_anchor_offset = Some(cursor);
        }
    }

    /// 当前光标是否仍落在锚点词的替换区间内（含两端）。锚点词即补全前缀词：
    /// 以锚点为中心向两侧扩展的 word 边界，与补全替换范围一致。区间外即关闭浮层。
    fn cursor_within_completion_anchor(&self, cursor: usize) -> bool {
        let Some(anchor) = self.completion_anchor_offset else {
            return false;
        };
        let len = self.buffer.len();
        let anchor = anchor.min(len);
        // 向左扩展到词首：连续 word_char 字符的起点。
        let text = self.text();
        let mut word_start = anchor;
        while word_start > 0 {
            let prev = self.buffer.prev_char_boundary(word_start);
            if prev == word_start {
                break;
            }
            if let Some(c) = text[prev..word_start].chars().next_back() {
                if !self.word_char(c) {
                    break;
                }
            }
            word_start = prev;
        }
        // 向右扩展到词末：连续 word_char 字符的终点（排除 exit 前的非边界）。
        let mut word_end = anchor;
        while word_end < len {
            let next_ = self.buffer.next_char_boundary(word_end);
            if next_ == word_end {
                break;
            }
            if let Some(c) = text[word_end..next_].chars().next() {
                if !self.word_char(c) {
                    break;
                }
            }
            word_end = next_;
        }
        cursor >= word_start && cursor <= word_end
    }

    /// 派生当前补全 session 生命周期状态（DM-708），用于划分日志。
    fn session_state(&self) -> CompletionSession {
        if self.completion_loading {
            CompletionSession::Triggering
        } else if self.completion_visible {
            if self.completion_continuation.is_some() {
                CompletionSession::Retriggering
            } else {
                CompletionSession::Active
            }
        } else {
            CompletionSession::Idle
        }
    }

    /// 隐藏补全列表。
    pub(crate) fn hide_completion(&mut self, cx: &mut Context<Self>) {
        // DM-703：隐藏即 bump session，令 in-flight 旧任务判废。
        self._completion_session_token.cancel();
        // F005：隐藏同时判废 in-flight 详情任务，清空右侧详情。
        self._completion_doc_token.cancel();
        self._completion_doc_task = None;
        self.completion_doc_state = None;
        self.completion_visible = false;
        self.completion_loading = false;
        self.completion_scroll_handle.set_offset(gpui::Point::default());
        self.completion_anchor_offset = None;
        self.completion_width = None;
        cx.notify();
    }

    /// 把光标定位到 `offset`（字节）并请求重绘，供宿主「跳转到某条语句」使用。
    /// 当前 `ensure_cursor_visible` 为空实现，故此处仅移动光标/选区。
    pub(crate) fn reveal_offset(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.buffer.len());
        self.selection = Selection::new(offset, offset);
        cx.notify();
    }

    // ------------------------------------------------------------ 查找

    /// 打开查找面板（宿主负责渲染 hosting 面板）。
    pub(crate) fn open_find(&mut self, cx: &mut Context<Self>) {
        self.find_state.open = true;
        // 未设置过查找词时，默认取选中文本作为查找词。
        if self.find_state.query.is_empty() {
            if let Some(sel) = self.selected_text() {
                self.find_state.query = sel;
            }
        }
        cx.notify();
    }

    /// 关闭查找面板。
    pub(crate) fn close_find(&mut self, cx: &mut Context<Self>) {
        self.find_state.open = false;
        self.find_state.found = false;
        self.find_state.current_match = None;
        cx.notify();
    }

    /// 设置查找词，并复位上一次命中的位置。
    pub(crate) fn set_find_query(&mut self, query: &str, cx: &mut Context<Self>) {
        self.find_state.query = query.to_string();
        self.find_state.search_start = 0;
        self.find_state.current_match = None;
        cx.notify();
    }

    /// 设置替换词。
    pub(crate) fn set_find_replace_text(&mut self, replace: &str, cx: &mut Context<Self>) {
        self.find_state.replace_text = replace.to_string();
        cx.notify();
    }

    /// 设置查找选项（match_case / whole_word / regex）。
    #[allow(dead_code)] // 查找选项面板未接入，随 FindOption 一并预留。
    pub(crate) fn set_find_option(
        &mut self,
        option: FindOption,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        match option {
            FindOption::MatchCase => self.find_state.match_case = enabled,
            FindOption::WholeWord => self.find_state.whole_word = enabled,
            FindOption::Regex => self.find_state.regex = enabled,
        }
        cx.notify();
    }

    /// 查找下一个匹配：从当前光标位置向后搜索。
    pub(crate) fn find_next(&mut self, cx: &mut Context<Self>) {
        if self.find_state.query.is_empty() {
            return;
        }
        self.find_internal(1, cx);
    }

    /// 查找上一个匹配：从当前光标位置向前搜索。
    pub(crate) fn find_previous(&mut self, cx: &mut Context<Self>) {
        if self.find_state.query.is_empty() {
            return;
        }
        self.find_internal(-1, cx);
    }

    /// 内部查找：按方向（+1 向后 / -1 向前）在当前文本中定位下一个匹配。
    fn find_internal(&mut self, direction: isize, cx: &mut Context<Self>) {
        let text = self.buffer.to_string();
        let query = &self.find_state.query;
        if query.is_empty() || text.is_empty() {
            self.find_state.found = false;
            cx.notify();
            return;
        }
        let cursor = self
            .find_state
            .current_match
            .map(|r| r.end)
            .unwrap_or_else(|| self.cursor_offset())
            .min(text.len());
        let matches = self.find_matches_cached(&text, query);
        if matches.is_empty() {
            self.find_state.found = false;
            self.find_state.current_match = None;
            cx.notify();
            return;
        }
        // 选择第一个严格位于当前位置之后（方向>0）或之前（方向<0）的匹配。
        let selected = if direction > 0 {
            matches
                .iter()
                .copied()
                .find(|r| r.start >= cursor)
                .unwrap_or_else(|| matches[0])
        } else {
            matches
                .iter()
                .rev()
                .copied()
                .find(|r| r.end <= cursor)
                .unwrap_or_else(|| matches[matches.len() - 1])
        };
        self.find_state.found = true;
        self.find_state.current_match = Some(selected);
        self.selection = Selection::new(selected.start, selected.end);
        self.ensure_cursor_visible();
        cx.emit(EditorEvent::SelectionChanged(self.selection.clone()));
        cx.notify();
    }

    /// 替换当前命中（如果存在）并继续查找下一个。
    #[allow(dead_code)] // 查找替换流程未接入宿主，保留该编辑能力。
    pub(crate) fn find_replace_current(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self.find_state.current_match {
            if m.start <= self.buffer.len() && m.end <= self.buffer.len() {
                let replace = self.find_state.replace_text.clone();
                let new_cursor = m.start + replace.len();
                self.selection = Selection::new(m.start, m.end);
                self.apply_edit(&replace, new_cursor, new_cursor, true, cx);
            }
        }
        self.new_find_next_after_edit(cx);
    }

    /// 替换全部匹配。
    #[allow(dead_code)] // 查找替换流程未接入宿主，保留该编辑能力。
    pub(crate) fn find_replace_all(&mut self, cx: &mut Context<Self>) {
        let text = self.buffer.to_string();
        let query = self.find_state.query.clone();
        if query.is_empty() || text.is_empty() {
            return;
        }
        // 构造一次结果并作为单个编辑提交，避免每个匹配都调度 syntax/diagnostic/completion。
        let matches = self.find_matches_cached(&text, &query);
        if matches.is_empty() {
            return;
        }
        let replace = self.find_state.replace_text.clone();
        let replaced_len = text.len()
            + matches
                .iter()
                .map(|m| replace.len().saturating_sub(m.end.saturating_sub(m.start)))
                .sum::<usize>();
        let mut output = String::with_capacity(replaced_len);
        let mut cursor = 0;
        for m in matches {
            if m.start > text.len() || m.end > text.len() || m.start < cursor {
                continue;
            }
            output.push_str(&text[cursor..m.start]);
            output.push_str(&replace);
            cursor = m.end;
        }
        output.push_str(&text[cursor..]);
        let old_cursor = self.cursor_offset().min(output.len());
        self.selection = Selection::new(0, text.len());
        self.apply_edit(&output, old_cursor, old_cursor, false, cx);
        self.find_state.current_match = None;
        self.find_state.found = false;
        cx.notify();
    }

    /// 编辑后把查找起点复位，使下一次“查找下一个”从当前光标处开始。
    #[allow(dead_code)] // 随查找替换流程一并预留。
    fn new_find_next_after_edit(&mut self, cx: &mut Context<Self>) {
        self.find_state.search_start = self.cursor_offset();
        self.find_next(cx);
    }

    /// 查找所有匹配区间（字节）。正则/整词/大小写由 find_state 控制。
    fn find_matches_cached(&self, text: &str, query: &str) -> Vec<CoreRange> {
        let key_matches = self.find_matches_cache.borrow();
        if let Some(cache) = key_matches.as_ref().filter(|cache| {
            cache.version == self.buffer.version()
                && cache.query == query
                && cache.match_case == self.find_state.match_case
                && cache.whole_word == self.find_state.whole_word
                && cache.regex == self.find_state.regex
        }) {
            return (*cache.matches).clone();
        }
        drop(key_matches);
        let matches = self.find_matches_uncached(text, query);
        *self.find_matches_cache.borrow_mut() = Some(FindMatchesCache {
            version: self.buffer.version(),
            query: query.to_string(),
            match_case: self.find_state.match_case,
            whole_word: self.find_state.whole_word,
            regex: self.find_state.regex,
            matches: Arc::new(matches.clone()),
        });
        matches
    }

    fn find_matches_uncached(&self, text: &str, query: &str) -> Vec<CoreRange> {
        let mut matches = Vec::new();
        if query.is_empty() {
            return matches;
        }
        // 正则模式：以邮箱字面量转义失败时不崩溃，简单实现仅支持普通文本。
        if self.find_state.regex {
            return self.find_matches_regex(text, query);
        }
        let needle = if self.find_state.match_case {
            query.to_string()
        } else {
            query.to_lowercase()
        };
        let hay = if self.find_state.match_case {
            text.to_string()
        } else {
            text.to_lowercase()
        };
        let needle_bytes = needle.as_bytes();
        let hay_bytes = hay.as_bytes();
        let mut start = 0usize;
        while start + needle_bytes.len() <= hay_bytes.len() {
            if &hay_bytes[start..start + needle_bytes.len()] == needle_bytes {
                let word_ok = !self.find_state.whole_word
                    || is_word_boundary(text, start, start + needle_bytes.len());
                if word_ok {
                    matches.push(CoreRange::new(start, start + needle_bytes.len()));
                }
                start += needle_bytes.len();
            } else {
                start += 1;
            }
        }
        matches
    }

    /// 正则查找；匹配结果直接使用 regex crate 的 UTF-8 byte offsets。
    fn find_matches_regex(&self, text: &str, _query: &str) -> Vec<CoreRange> {
        let Ok(regex) = regex::RegexBuilder::new(self.find_state.query.as_str())
            .case_insensitive(!self.find_state.match_case)
            .build()
        else {
            return Vec::new();
        };
        regex
            .find_iter(text)
            .filter_map(|m| {
                (!self.find_state.whole_word
                    || is_word_boundary(text, m.start(), m.end()))
                    .then_some(CoreRange::new(m.start(), m.end()))
            })
            .collect()
    }

    /// 查找面板是否打开。
    pub(crate) fn find_open(&self) -> bool {
        self.find_state.open
    }


    // ------------------------------------------------------------ 语法 / 诊断 / hover / 执行

    /// 安排一次语法解析。
    ///
    /// 携带本次编辑的脏区间（`change` → `InputEdit`）交给 provider；解析在后台
    /// 异步进行，结果落库前先做版本校验，过期结果直接丢弃，避免旧编辑覆盖新语法
    /// （见设计 5.3 / 六）。渲染层随后按可见行读取 `self.syntax.highlights`。
    fn request_syntax(&mut self, change: &TextChange, cx: &mut Context<Self>) {
        let Some(syntax) = self.providers.syntax.clone() else {
            return;
        };
        // 新编辑到来时丢弃尚未开始的旧解析任务，避免连续输入堆积后台工作。
        self._syntax_task = None;
        let snapshot = self.buffer.snapshot();
        let version = snapshot.version();
        let text_bytes = snapshot.len();
        let editor_id = self.perf_editor_id;
        let edit_id = self.perf_edit_id;
        // DM-620/624：解析前消费父层注入，构建语法层树供按 offset 路由。
        // 单语言（无注入）时为空层树，行为不变零开销。
        self.syntax_layers = SyntaxLayerTree::from_injections(
            syntax.injections(&snapshot, CoreRange::new(0, snapshot.len())),
        );
        tracing::debug!(
            target: "gdb_editor_perf",
            op = "syntax_schedule",
            editor_id,
            edit_id,
            buffer_version = version,
            text_bytes,
        );
        let small_edit = change.new_text.len() <= 256
            && change.old_range.end.saturating_sub(change.old_range.start) <= 256;
        let refinement = change.version == version
            && change.old_range.is_empty()
            && change.new_text.is_empty();
        let debounce_ms = if refinement {
            500
        } else if small_edit {
            PROVIDER_DEBOUNCE_MS
        } else {
            150
        };
        let input_edit = if change.full_document {
            InputEdit::full_document(change.new_text.clone(), change.version)
        } else {
            InputEdit::new(change.old_range, change.new_text.clone(), change.version)
        };
        let mut input_edit = input_edit;
        input_edit.edit_id = edit_id;
        let task_id = NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed);
        let task = cx.spawn(async move |this, cx| {
            // Tree-sitter/语法 provider 属于 CPU 密集任务，不能在 GPUI executor 内同步运行。
            let task_started = Instant::now();
            smol::Timer::after(Duration::from_millis(debounce_ms)).await;
            let current = this
                .update(cx, |this, _| this.buffer.version() == version)
                .unwrap_or(false);
            if !current {
                tracing::debug!(
                    target: "gdb_editor_perf",
                    op = "syntax_discarded",
                        task_outcome = TaskOutcome::ResultDiscarded.as_str(),
                    editor_id,
                    edit_id,
                    task_id,
                    phase = "debounce",
                    elapsed_us = task_started.elapsed().as_micros() as u64,
                    buffer_version = version,
                    text_bytes,
                );
                return;
            }
            let result = cx
                .background_spawn(async move { syntax.parse(&snapshot, input_edit).await })
                .await;
            let needs_refinement = result.needs_refinement;
            let accepted = this
                .update(cx, |this, cx| {
                // 版本校验：仅当结果对应的是当前 buffer 版本才采纳；
                // 解析期间发生了新编辑则丢弃（按需会再次触发新解析）。
                if this.buffer.snapshot().version() != version
                    || result.buffer_version != version
                    // latest-wins provider uses an empty result as a cancellation marker.
                    || (result.highlights.is_empty() && result.dirty_ranges.is_empty())
                {
                    tracing::debug!(
                        target: "gdb_editor_perf",
                        op = "syntax_discarded",
                            task_outcome = TaskOutcome::ResultDiscarded.as_str(),
                        editor_id,
                        edit_id,
                        task_id,
                        elapsed_us = task_started.elapsed().as_micros() as u64,
                        buffer_version = version,
                        text_bytes,
                    );
                    return false;
                }
                let highlight_count = result.highlights.len();
                // 采纳：把 provider 的 patch 合并进单一 HighlightStore（DM-400/401）。
                //   - dirty_ranges 为空         → 全量结果：`from_highlights` 复建主存储（仅此一次全量构造）
                //   - 单 dirty 区间（主流增量） → 对当前 store `apply_range`，未相交 chunk Arc 共享，
                //     provider 的 statement 路径已把 `highlights` 作用域限定在该区间内。
                // store 在每次编辑时已由 `preserve_syntax_after_edit` 的 `apply_edit` 平移到位，版本身份一致。
                let base = match &this.syntax {
                    Some(s) if s.buffer_version == result.buffer_version => {
                        (*s.highlight_store).clone()
                    }
                    _ => fluxdb_editor_core::HighlightStore::default(),
                };
                let highlight_store = if result.dirty_ranges.is_empty() {
                    fluxdb_editor_core::HighlightStore::from_highlights(
                        Arc::new(result.highlights.clone()),
                        text_bytes,
                    )
                } else if result.dirty_ranges.len() == 1 {
                    base.apply_range(result.dirty_ranges[0], &result.highlights)
                } else {
                    // 多 dirty 区间（罕见）：视作全量重建，保证正确。
                    fluxdb_editor_core::HighlightStore::from_highlights(
                        Arc::new(result.highlights.clone()),
                        text_bytes,
                    )
                };
                let highlights = Arc::new(result.highlights);
                this.syntax = Some(SyntaxSnapshot {
                    buffer_version: result.buffer_version,
                    highlight_store: Arc::new(highlight_store),
                    highlights,
                });
                // 语法任务同时预热了 adapter 的 CodeLens；清掉布局阶段可能缓存的空结果。
                this.code_lens_cache.borrow_mut().take();
                this.code_lens_visual_rows_cache.borrow_mut().take();
                this.shaped_line_cache.borrow_mut().clear();
                cx.notify();
                tracing::debug!(
                    target: "gdb_editor_perf",
                    op = "syntax_result",
                    editor_id,
                    edit_id,
                    task_id,
                    elapsed_us = task_started.elapsed().as_micros() as u64,
                    buffer_version = version,
                    text_bytes,
                    highlight_count,
                );
                true
            })
            .unwrap_or(false);
            if accepted && needs_refinement {
                let _ = this.update(cx, |this, cx| {
                    if this.buffer.version() != version {
                        return;
                    }
                    tracing::debug!(
                        target: "gdb_editor_perf",
                        op = "syntax_refinement_schedule",
                        editor_id,
                        edit_id,
                        task_id,
                        buffer_version = version,
                        text_bytes,
                    );
                    // 同版本空变更强制 provider 走正式解析分支；新编辑会通过版本号
                    // 和 provider latest-wins 令牌取消这次后台精解析。
                    let refinement = TextChange::new(
                        CoreRange::new(0, 0),
                        String::new(),
                        version,
                    );
                    this.request_syntax(&refinement, cx);
                });
            }
        });
        self._syntax_task = Some(task);
    }

    fn request_diagnostics(&mut self, change: &TextChange, cx: &mut Context<Self>) {
        let Some(diag) = self.providers.diagnostics.clone() else {
            return;
        };
        let snapshot = self.buffer.snapshot();
        let version = snapshot.version();
        let text_bytes = snapshot.len();
        let editor_id = self.perf_editor_id;
        let edit_id = self.perf_edit_id;
        let task_id = NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed);
        let input_edit = if change.full_document {
            InputEdit::full_document(change.new_text.clone(), change.version)
        } else {
            InputEdit::new(change.old_range, change.new_text.clone(), change.version)
        };
        let mut input_edit = input_edit;
        input_edit.edit_id = edit_id;
        let debounce_ms = if text_bytes > 512 * 1024 {
            LARGE_DOCUMENT_DIAGNOSTIC_DEBOUNCE_MS
        } else {
            PROVIDER_DEBOUNCE_MS
        };
        let task = cx.spawn(async move |this, cx| {
            let task_started = Instant::now();
            smol::Timer::after(Duration::from_millis(debounce_ms)).await;
            // 连续输入时，旧诊断任务在进入后台前直接退出，避免每个按键都全文解析。
            let current = this
                .update(cx, |this, _| this.buffer.version() == version)
                .unwrap_or(false);
            if !current {
                tracing::debug!(
                    target: "gdb_editor_perf",
                    op = "diagnostics_discarded",
                        task_outcome = TaskOutcome::ResultDiscarded.as_str(),
                    editor_id,
                    edit_id,
                    task_id,
                    phase = "debounce",
                    elapsed_us = task_started.elapsed().as_micros() as u64,
                    buffer_version = version,
                    text_bytes,
                );
                return;
            }
            let results = cx
                .background_spawn(async move {
                    diag.diagnostics_incremental(&snapshot, input_edit)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.buffer.snapshot().version() != version {
                    tracing::debug!(
                        target: "gdb_editor_perf",
                        op = "diagnostics_discarded",
                            task_outcome = TaskOutcome::ResultDiscarded.as_str(),
                        editor_id,
                        edit_id,
                        task_id,
                        elapsed_us = task_started.elapsed().as_micros() as u64,
                        buffer_version = version,
                        text_bytes,
                    );
                    return;
                }
                let diagnostic_count = results.len();
                this.diagnostics = results;
                this.diagnostic_line_index = fluxdb_editor_core::build_range_line_index(
                    &this.buffer.snapshot(),
                    this.diagnostics.iter().map(|diagnostic| diagnostic.range),
                );
                cx.notify();
                tracing::debug!(
                    target: "gdb_editor_perf",
                    op = "diagnostics_result",
                    editor_id,
                    edit_id,
                    task_id,
                    elapsed_us = task_started.elapsed().as_micros() as u64,
                    buffer_version = version,
                    text_bytes,
                    diagnostic_count,
                );
            });
        });
        self._diagnostics_task = Some(task);
    }

    fn request_hover(&mut self, point: Point, cx: &mut Context<Self>) {
        let Some(hover) = self.providers.hover.clone() else {
            return;
        };
        let snapshot = self.buffer.snapshot();
        let version = snapshot.version();
        // 每次请求先 bump：使旧的 in-flight hover 失效（DM-603）。
        let request_id = self._hover_token.request_id();
        let hover_token = self._hover_token.clone();
        let editor_id = self.perf_editor_id;
        let edit_id = self.perf_edit_id;
        let task_id = NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed);
        self.hover_loading = true;
        self.hovered_point = Some(point);
        let task = cx.spawn(async move |this, cx| {
            let task_started = Instant::now();
            let content = hover.hover(&snapshot, point);
            let _ = this.update(cx, |this, cx| {
                if this.buffer.snapshot().version() != version
                    || !hover_token.check(request_id)
                {
                    tracing::debug!(
                        target: "gdb_editor_perf",
                        op = "hover_discarded",
                        task_outcome = TaskOutcome::ResultDiscarded.as_str(),
                        elapsed_us = task_started.elapsed().as_micros() as u64,
                        editor_id,
                        edit_id,
                        task_id,
                        buffer_version = version,
                        request_id,
                        layer = "hover",
                    );
                    return;
                }
                this.hover_content = content;
                this.hover_loading = false;
                cx.notify();
                tracing::debug!(
                    target: "gdb_editor_perf",
                    op = "hover_result",
                    task_outcome = TaskOutcome::ResultCommitted.as_str(),
                    elapsed_us = task_started.elapsed().as_micros() as u64,
                    editor_id,
                    edit_id,
                    task_id,
                    buffer_version = version,
                    request_id,
                    layer = "hover",
                    has_content = this.hover_content.is_some(),
                );
            });
        });
        self._hover_task = Some(task);
    }

    pub(crate) fn request_execution_selection(&mut self, cx: &mut Context<Self>) {
        let Some(exec) = self.providers.execution.clone() else {
            return;
        };
        let snapshot = self.buffer.snapshot();
        let selection = self.selection;
        let units = exec.execution_units(&snapshot, selection);
        for unit in units {
            // 事件只传中性范围与模式；语句文本由接入层/宿主按 range 从编辑器读取，避免事件携带全文。
            cx.emit(EditorEvent::Execute {
                range: unit.range,
                mode: unit.mode,
            });
        }
    }

    /// 以动作携带的 `mode` 请求执行（Run/Select/Explain 分派入口，整改 6.1/6.2）。
    ///
    /// `ExecuteQueryShortcut.mode` 表达的是用户本次操作的真实意图，必须原样透传，
    /// 不能再用 `execution_units` 按文本探测的模式覆盖：SELECT 语句在 cmd-shift-r(Select)
    /// 或 cmd-shift-e(Explain) 下也应路由到对应路径。计算执行单元仍由 adapter 完成。
    pub(crate) fn request_execution(&mut self, mode: ExecuteMode, cx: &mut Context<Self>) {
        let Some(exec) = self.providers.execution.clone() else {
            return;
        };
        let snapshot = self.buffer.snapshot();
        let selection = self.selection;
        let units = exec.execution_units(&snapshot, selection);
        for unit in units {
            cx.emit(EditorEvent::Execute {
                range: unit.range,
                mode,
            });
        }
    }

    pub(crate) fn toggle_line_comment(&mut self, cx: &mut Context<Self>) {
        if self.profile.read_only {
            return;
        }
        let Some(language) = self.providers.language.clone() else {
            return;
        };
        // 无行注释定义的语言不支持整行注释。
        let Some(prefix) = language.line_comment() else {
            return;
        };
        let cursor = self.cursor_offset();
        let point = self.buffer.offset_to_point(cursor);
        let line_start = self.buffer.line_start(point.row);
        let line = self.buffer.line_text(point.row);
        let lead = line.len() - line.trim_start().len();
        // 根据当前行是否已带注释前缀，决定替换区间与替换文本。
        let (edit_range, insert, new_cursor) = if line[lead..].starts_with(&prefix) {
            (
                CoreRange::new(line_start + lead, line_start + lead + prefix.len()),
                "",
                self.cursor_offset().saturating_sub(prefix.len()),
            )
        } else {
            (
                CoreRange::new(line_start + lead, line_start + lead),
                prefix,
                self.cursor_offset() + prefix.len(),
            )
        };
        let edit = self.buffer.edit(edit_range, insert, new_cursor, new_cursor, false);
        self.selection = Selection::point(new_cursor);
        self.rebuild_display();
        // 局部单行编辑，用增量变更驱动宿主同步，避免整文档 `to_string()`（DM-503）。
        if let Some(change) = edit.changes.first() {
            self.after_edit(change, cx);
        }
    }

    /// 行号区宽度（像素），由渲染阶段采用。
    pub(crate) fn line_number_width(&self, _window: &Window) -> gpui::Pixels {
        if !self.gutter_line_numbers && !self.profile.show_line_numbers {
            return gpui::px(0.);
        }
        gpui::px(self.line_digits() as f32 * 8.0 + EDITOR_GUTTER_GAP + EDITOR_FOLD_GUTTER)
            .max(gpui::px(EDITOR_MIN_GUTTER))
    }

    fn line_digits(&self) -> usize {
        self.buffer.line_count().max(1).to_string().len().max(2)
    }

    pub(crate) fn content_height(&self, window: &Window) -> gpui::Pixels {
        let line_count = self.display.visual_row_count().max(1) as f32;
        // 额外块高（CodeLens）行数来自 Block 摘要，不再重复 count(lens_rows)。
        let block_extra = self
            .display
            .block_total_rows()
            .saturating_sub(self.display.visual_row_count()) as f32;
        let line_height = f32::from(self.line_height(window));
        // Zed 默认 scroll_beyond_last_line=one_page：最后一行可以滚到视口顶部。
        // 首帧视口尚未布局时退回一行，避免凭空产生大块可滚空白。
        let overscroll = f32::from(self.scroll_handle.bounds().size.height).max(line_height);
        gpui::px(
            EDITOR_PADDING_Y * 2.
                + line_count * line_height
                + (line_count - 1.).max(0.) * EDITOR_LINE_GAP
                + block_extra * CODE_LENS_HEIGHT
                + if line_count > 1. { overscroll } else { 0. },
        )
    }

    pub(crate) fn content_width(&self, window: &Window) -> gpui::Pixels {
        if self.soft_wrap {
            return self.scroll_handle.bounds().size.width.max(gpui::px(1.));
        }
        let buffer_version = self.buffer.version();
        let text_len = self.buffer.len();
        let line_count = self.buffer.line_count();
        let font_size_bits = self.font_size.to_bits();
        if let Some(cache) = self.content_width_cache.borrow().as_ref()
            && cache.buffer_version == buffer_version
            && cache.text_len == text_len
            && cache.line_count == line_count
            && cache.font_size_bits == font_size_bits
        {
            return gpui::px(cache.width);
        }
        let char_width = measure_character_width(window, self.font_size);
        let longest = self.longest_line_width(window, char_width);
        let width = gpui::px(EDITOR_PADDING_X * 2.)
            + self.line_number_width(window)
            + gpui::px(EDITOR_CONTENT_GAP)
            + longest
            // 保留一个字符的末尾余量，避免光标/最后一个 glyph 被裁掉；不要按固定大块
            // overscroll，否则长文档会出现明显的右侧空白区。
            + gpui::px(char_width);
        let width = f32::from(width);
        *self.content_width_cache.borrow_mut() = Some(ContentWidthCache {
            buffer_version,
            text_len,
            line_count,
            font_size_bits,
            width,
        });
        gpui::px(width)
    }

    fn update_line_width_hint(&self, start: usize, inserted: &str) {
        let Some((mut max_columns, width)) = *self.line_width_hint.borrow() else {
            return;
        };
        let start_row = self.buffer.offset_to_point(start).row;
        let rows_to_check = inserted.bytes().filter(|byte| *byte == b'\n').count() + 2;
        let end_row = (start_row + rows_to_check).min(self.buffer.line_count());
        for row in start_row..end_row {
            max_columns = max_columns.max(Self::line_columns(&self.buffer.line_text(row), self.tab_width));
        }
        *self.line_width_hint.borrow_mut() = Some((max_columns, width));
    }

    fn line_columns(text: &str, tab_width: usize) -> usize {
        text.chars()
            .map(|character| if character == '\t' { tab_width } else { 1 })
            .sum()
    }

    fn longest_line_width(&self, window: &Window, char_width: f32) -> gpui::Pixels {
        if let Some((columns, measured_width)) = *self.line_width_hint.borrow() {
            return gpui::px(measured_width.max(columns as f32 * char_width));
        }
        let mut longest_columns = 0usize;
        let mut longest_text = String::new();
        for row in 0..self.buffer.line_count() {
            let text = self.buffer.line_text(row);
            let columns = Self::line_columns(&text, self.tab_width);
            if columns > longest_columns {
                longest_columns = columns;
                longest_text = text;
            }
        }
        if longest_text.is_empty() {
            return gpui::px(0.);
        }
        let run = TextRun {
            len: longest_text.len(),
            font: self.editor_font(),
            color: window.text_style().color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window.text_system().shape_line(
            SharedString::from(longest_text),
            self.font_size.into(),
            &[run],
            None,
        );
        // ponytail: 只 shape 列数最长的一行；混合宽字符且列数较短的行可能被低估，
        // 若需要精确支持这类文本，再升级为按行增量宽度缓存。
        let measured_width = f32::from(shaped.width);
        *self.line_width_hint.borrow_mut() = Some((longest_columns, measured_width));
        gpui::px(measured_width.max(longest_columns as f32 * char_width))
    }
}

// ---------------------------------------------------------------- 补全过滤

/// 只过滤、不重排（T084）：fluxdb-app SQL 补全已在 App 层按语义（匹配 tier → 意图相关度 →
/// 类型 → 标签）全局排序，返回即最终顺序。桌面层若再用 `match_score`（子序列跨度）或
/// label 二次重排，会把 App 的高置信度语义排名覆盖掉（例如 `cr` 时把 category_id 排到
/// created_at 之前）。故此处仅丢弃不匹配候选，保留 App 的排序；这同时让桌面驱动的
/// inject 路径与 App 路径的排序行为统一。
fn filter_completion_items_preserving_order(items: Vec<CompletionItem>, query: &str) -> Vec<CompletionItem> {
    items.into_iter().filter(|item| item.match_score(query).is_some()).collect()
}

// ---------------------------------------------------------------- 光标移动目标

#[derive(Clone, Copy)]
pub(crate) enum CursorMove {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Start,
    EndAll,
    PrevWord,
    NextWord,
}

// ---------------------------------------------------------------- 字体测量

/// 测量单个字符宽度（像素）。与旧 sql_editor 一致：用 shape_line 实测字符宽度，最小 7px。
pub(crate) fn measure_character_width(window: &Window, size: f32) -> f32 {
    let base_font = editor_font();
    let run = TextRun {
        len: 1,
        font: base_font,
        color: window.text_style().color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(SharedString::from("0"), size.into(), &[run], None);
    f32::from(line.width).max(7.0)
}

/// 编辑器等宽字体。与旧 sql_editor 的 `editor_font` 保持一致。
pub(crate) fn editor_font() -> gpui::Font {
    gpui::font(EDITOR_FONT)
}

impl Editor {
    pub(crate) fn editor_font(&self) -> gpui::Font {
        gpui::font(&self.font_name)
    }

    pub(crate) fn measure_character_width(&self, window: &Window) -> f32 {
        let run = TextRun {
            len: 1,
            font: self.editor_font(),
            color: window.text_style().color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(
            SharedString::from("0"),
            self.font_size.into(),
            &[run],
            None,
        );
        f32::from(line.width).max(7.0)
    }

    pub(crate) fn line_height(&self, _window: &Window) -> gpui::Pixels {
        gpui::px(self.line_height.max(self.font_size + 1.))
    }
}

#[cfg(test)]
mod word_motion_tests {
    use super::*;

    /// 参考实现：用全文字节数组复现原逻辑，作为局部行扫描结果的期望参照。
    fn prev_word_ref(text: &str, cursor: usize) -> usize {
        let bytes = text.as_bytes();
        let mut i = cursor.min(bytes.len());
        while i > 0 && bytes[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        while i > 0 && !bytes[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        i
    }

    fn next_word_ref(text: &str, cursor: usize) -> usize {
        let bytes = text.as_bytes();
        let mut i = cursor.min(bytes.len());
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        i.min(bytes.len())
    }

    #[test]
    fn word_motion_matches_fulltext_reference() {
        let texts = [
            "SELECT name FROM users;",
            "  leading spaces here",
            "   ",
            "",
            "one\ntwo\nthree",
            "alpha beta\ngamma   delta",
            "  a  \n  b  ",
            "中文 汉字 word",
            "a\tb c\nd e",
        ];
        for text in texts {
            let snap = EditorBuffer::new_from(text).snapshot();
            let len = text.len();
            // 采样多个光标位置，包括 0 / 末尾 / 若干中间字符边界。
            let mut cursors = vec![0usize, len];
            let mut byte = 0usize;
            let mut idx = 0usize;
            for c in text.chars() {
                if idx % 2 == 0 {
                    cursors.push(byte);
                }
                idx += 1;
                byte += c.len_utf8();
            }
            for cursor in cursors {
                assert_eq!(
                    prev_word_start_in_snap(&snap, cursor),
                    prev_word_ref(text, cursor),
                    "prev_word text={text:?} cursor={cursor}"
                );
                assert_eq!(
                    next_word_start_in_snap(&snap, cursor),
                    next_word_ref(text, cursor),
                    "next_word text={text:?} cursor={cursor}"
                );
            }
        }
    }
}

#[cfg(test)]
mod completion_filter_tests {
    use super::*;
    use fluxdb_editor_core::CompletionKind;

    /// T084：fluxdb-app 已在 App 层按语义全局排序，桌面层只过滤、不重排。
    /// `cr` 时应保留 App 的顺序（category_id 在 created_at 之前），并丢弃不匹配项。
    #[test]
    fn filter_preserves_app_semantic_order() {
        let items = vec![
            CompletionItem::new("category_id", CompletionKind::Column),
            CompletionItem::new("created_at", CompletionKind::Column),
            CompletionItem::new("name", CompletionKind::Column),
        ];
        let filtered = filter_completion_items_preserving_order(items, "cr");
        let labels: Vec<_> = filtered.iter().map(|i| i.label.as_str()).collect();
        // category_id 与 created_at 都子序列匹配 "cr"，name 不匹配被丢弃；顺序保持不变。
        assert_eq!(labels, vec!["category_id", "created_at"]);
    }

    #[test]
    fn filter_empty_query_keeps_all_in_order() {
        let items = vec![
            CompletionItem::new("b", CompletionKind::Column),
            CompletionItem::new("a", CompletionKind::Column),
        ];
        let filtered = filter_completion_items_preserving_order(items, "");
        let labels: Vec<_> = filtered.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["b", "a"]);
    }
}
