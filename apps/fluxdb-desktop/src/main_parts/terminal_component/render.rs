// 终端画布渲染：把 fluxdb-core 的 TermGrid 画到 GPUI。
//
// 复用 sql_editor 的自定义 Element 模式（request_layout / prepaint / paint）：
//   - prepaint 阶段按实测 cell 尺寸计算 cols/rows，并据此 reflow grid + resize PTY。
//   - paint 阶段按逐列 fg/bg/bold 上色绘制每一行，并在底部绘制状态栏与危险命令确认条。
//   - 通过 `window.handle_input` 挂上输入 handler，使可打印字符/IME 进入 EntityInputHandler。

use fluxdb_core::terminal::{TermRowView, byte_cursor_to_cell};
use gpui::TextAlign;

/// 终端用等宽字体与字号（与需要宽字符显示的 grid 对齐）。
pub(crate) const TERMINAL_FONT: &str = "Menlo";
pub(crate) const TERMINAL_FONT_SIZE: f32 = 12.0;

/// 终端前景/背景色（随主题深浅取两套近似配色）。
#[derive(Clone, Copy)]
struct TermColors {
    bg: gpui::Rgba,
    default_fg: gpui::Rgba,
    completion_hint_fg: gpui::Rgba,
    status_bg: gpui::Rgba,
    status_fg: gpui::Rgba,
    danger_bg: gpui::Rgba,
    danger_fg: gpui::Rgba,
    /// 选区高亮（半透明铺在文本下层）。
    selection_bg: gpui::Rgba,
}

impl TermColors {
    fn for_theme(is_dark: bool) -> Self {
        // gpui 0.2.2 的 `rgb` 只接受单个打包的 0xRRGGBB 值。
        if is_dark {
            TermColors {
                bg: rgb(0x101216),
                default_fg: rgb(0xd8dce0),
                completion_hint_fg: rgb(0x73777d),
                status_bg: rgb(0x1f2329),
                status_fg: rgb(0x9ca3aa),
                danger_bg: rgb(0x4a1d1d),
                danger_fg: rgb(0xffb4b4),
                selection_bg: rgb(0x33527a),
            }
        } else {
            TermColors {
                bg: rgb(0xffffff),
                default_fg: rgb(0x1f2428),
                completion_hint_fg: rgb(0x9aa0a6),
                status_bg: rgb(0xeff1f4),
                status_fg: rgb(0x5c6269),
                danger_bg: rgb(0xfbe4e4),
                danger_fg: rgb(0x9b1c1c),
                selection_bg: rgb(0xbcd7f0),
            }
        }
    }
}

// 终端视图：外层 focusable div 承载 key context + 动作转发，内层 TerminalCanvas 负责绘制。
impl Render for TerminalComponent {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus_handle = self.focus_handle.clone();
        let component = cx.entity();
        div()
            .id(("terminal", component.entity_id()))
            .flex_1()
            .size_full()
            .relative()
            .overflow_hidden()
            .key_context(TERMINAL_CONTEXT)
            .track_focus(&focus_handle)
            .tab_index(0)
            .on_mouse_down(MouseButton::Left, {
                let focus_handle = focus_handle.clone();
                move |_, window, _cx| {
                    focus_handle.focus(window, _cx);
                }
            })
            .on_action(window.listener_for(&component, TerminalComponent::enter))
            .on_action(window.listener_for(&component, TerminalComponent::escape))
            .on_action(window.listener_for(&component, TerminalComponent::ctrl_c))
            .on_action(window.listener_for(&component, TerminalComponent::ctrl_d))
            .on_action(window.listener_for(&component, TerminalComponent::backspace))
            .on_action(window.listener_for(&component, TerminalComponent::delete))
            .on_action(window.listener_for(&component, TerminalComponent::move_left))
            .on_action(window.listener_for(&component, TerminalComponent::move_right))
            .on_action(window.listener_for(&component, TerminalComponent::move_up))
            .on_action(window.listener_for(&component, TerminalComponent::move_down))
            .on_action(window.listener_for(&component, TerminalComponent::move_home))
            .on_action(window.listener_for(&component, TerminalComponent::move_end))
            .on_action(window.listener_for(&component, TerminalComponent::tab))
            .on_action(window.listener_for(&component, TerminalComponent::shift_tab))
            .on_action(window.listener_for(&component, TerminalComponent::copy))
            .on_action(window.listener_for(&component, TerminalComponent::paste))
            .on_action(window.listener_for(&component, TerminalComponent::clear))
            // 鼠标：左键按下开始选区（并聚焦），拖动更新，抬起/移出结束；滚轮滚动回看。
            .on_mouse_down(MouseButton::Left, {
                let component = component.clone();
                move |event, _window, cx| {
                    component.update(cx, |comp, cx| {
                        comp.begin_mouse_selection(event.position);
                        cx.notify();
                    });
                }
            })
            .on_mouse_move({
                let component = component.clone();
                move |event, _window, cx| {
                    component.update(cx, |comp, _| comp.update_mouse_selection(event.position));
                }
            })
            .on_mouse_up(MouseButton::Left, {
                let component = component.clone();
                move |_, _window, cx| {
                    component.update(cx, |comp, _| comp.end_mouse_selection());
                }
            })
            .on_mouse_up_out(MouseButton::Left, {
                let component = component.clone();
                move |_, _window, cx| {
                    component.update(cx, |comp, _| comp.end_mouse_selection());
                }
            })
            .on_scroll_wheel({
                let component = component.clone();
                move |event, _window, cx| {
                    let delta = event.delta.pixel_delta(px(1.0));
                    component.update(cx, |comp, cx| {
                        comp.scroll_by_delta(f32::from(delta.y));
                        cx.notify();
                    });
                    cx.stop_propagation();
                }
            })
            .child(TerminalCanvas {
                component: component.clone(),
            })
            .when(self.completion_active(), |this| {
                // 补全浮层：叠加在终端面上、贴近输入行，供用户用 Tab/Shift-Tab 选、Enter 应用、Esc 关闭。
                let items = self.completion_items.clone();
                let selected = self.completion_index;
                this.child(completion_popup(
                    &items,
                    selected,
                    ComponentTheme::global(cx).radius_lg,
                ))
            })
    }
}

/// 渲染补全候选浮层（绝对定位于终端左下角，覆盖输入行区域）。
fn completion_popup(
    items: &[TerminalCompletionItem],
    selected: Option<usize>,
    radius: gpui::Pixels,
) -> impl IntoElement {
    let rows: Vec<_> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = Some(i) == selected;
            let label = item.label.clone();
            let detail = item.detail.clone();
            div()
                .px_2()
                .py_0p5()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .when(is_selected, |this| this.bg(rgb(0x33527a)))
                .child(
                    div()
                        .text_color(if is_selected {
                            rgb(0xffffff)
                        } else {
                            rgb(0x9aa5b1)
                        })
                        .child(label),
                )
                .when(detail.is_some(), |this| {
                    this.child(
                        div()
                            .text_color(rgb(0x6b7280))
                            .text_sm()
                            .child(detail.unwrap_or_default()),
                    )
                })
        })
        .collect();

    div()
        .id("terminal-completion-popup")
        .absolute()
        .left_0()
        .bottom_2()
        .min_w(px(320.0))
        .max_h(px(260.0))
        .overflow_y_scrollbar()
        .bg(rgb(0x1a1b1e))
        .shadow_lg()
        .rounded(radius)
        .border_1()
        .border_color(rgb(0x3a3d42))
        .px_1()
        .py_1()
        .child(div().flex().flex_col().children(rows))
}

/// 终端画布 Element。
pub(crate) struct TerminalCanvas {
    pub component: Entity<TerminalComponent>,
}

/// prepaint 阶段缓存：cell 尺寸与布局参数。
struct TerminalLayout {
    cell_w: Pixels,
    line_h: Pixels,
    status_h: Pixels,
    /// 全角字符的绘图字号缩放比（使 CJK 字形前进恰为 2 倍 cell_w，光标按格对齐不发生漂移）。
    cjk_scale: f32,
}

impl IntoElement for TerminalCanvas {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalCanvas {
    type RequestLayoutState = ();
    type PrepaintState = TerminalLayout;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style::default();
        // 终端元素填满整个可用区域（宽高均取 100%）。
        style.size = gpui::Size::full();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let cell_w = measure_terminal_cell(window, cx);
        let line_h = px(TERMINAL_FONT_SIZE + 3.0);
        let status_h = px(24.0);
        // 全角缩放到 2*cell_w，避免 CJK 字形窄于其占位格导致光标相对字形逐渐漂移。
        let cjk_scale = {
            let cjk_w = f32::from(measure_cjk_cell(window, cx)).max(1.0);
            (2.0 * f32::from(cell_w)) / cjk_w
        };

        let (grid_cols, grid_rows) = {
            let w = bounds.size.width;
            let h = bounds.size.height;
            let cols = (f32::from(w) / f32::from(cell_w)).floor().max(2.0) as u16;
            let rows = ((f32::from(h) - f32::from(status_h)) / f32::from(line_h))
                .floor()
                .max(1.0) as u16;
            (cols, rows)
        };

        // 记录布局参数供鼠标事件做像素→cell 换算；尺寸变化时 reflow grid 并 resize PTY。
        let content_h = f32::from(bounds.size.height) - f32::from(status_h);
        self.component.update(cx, |comp, _cx| {
            comp.set_layout(
                f32::from(cell_w),
                f32::from(line_h),
                f32::from(status_h),
                content_h,
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
            );
            if comp.grid.cols() != grid_cols || comp.grid.rows() != grid_rows {
                comp.grid.resize(grid_cols, grid_rows);
                if let Some(pty) = comp.transport.as_mut() {
                    pty.resize(grid_cols, grid_rows);
                }
            }
        });

        TerminalLayout {
            cell_w,
            line_h,
            status_h,
            cjk_scale,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let colors = TermColors::for_theme(cx.theme().is_dark());
        // 正文绘制统一相对终端画布的左上角：加上 bounds.origin 偏移，
        // 否则终端画布不在窗口 (0,0) 时(左侧有树/抽屉等)正文会被画到画布左侧之外而被裁剪，导致“有字看不见”。
        let left = f32::from(bounds.origin.x);

        // 背景。
        window.paint_quad(fill(bounds, colors.bg));

        // 挂输入 handler：可打印字符 / IME 进入 TerminalComponent（EntityInputHandler）。
        let focus_handle = self.component.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.component.clone()),
            cx,
        );

        // 读 grid 快照。
        let (visible_lines, cursor, following) = {
            let comp = self.component.read(cx);
            (
                comp.grid.visible_lines(),
                comp.grid.cursor(),
                comp.grid.following_bottom(),
            )
        };

        // 展示提示符沿用 redis-cli 原生的 `host:port[db]>`，历史命令行与当前行保持一致。
        let (prompt_state, native_prompt, display_prompt) = {
            let comp = self.component.read(cx);
            (
                comp.state,
                comp.adapter.prompt(),
                comp.adapter.display_prompt(),
            )
        };
        let show_display_prompt = following
            && matches!(
                prompt_state,
                fluxdb_core::terminal::TerminalSessionState::PtyRunning
                    | fluxdb_core::terminal::TerminalSessionState::Ready
                    | fluxdb_core::terminal::TerminalSessionState::AwaitingConfirmation
            );
        let current_line = visible_lines.get(cursor.y as usize).map(String::as_str);
        let current_prompt_line = show_display_prompt
            .then(|| {
                current_line
                    .and_then(|line| normalize_prompt_line(line, &native_prompt, &display_prompt))
            })
            .flatten();
        let display_cursor_x =
            if let Some((_, native_prefix_len, input_len)) = current_prompt_line.as_ref() {
                // 光标 x 是 redis 回显的**字节**位置；先吸附到最近字符边界的**单元列**，
                // 再减去提示符列得到输入内单元列光标。不然含宽字符时光标会落进字内/偏位。
                let row_cells: Vec<char> = current_line
                    .map(|s| s.chars().collect())
                    .unwrap_or_default();
                let input_cursor = byte_cursor_to_cell(&row_cells, cursor.x as usize)
                    .saturating_sub(*native_prefix_len);
                display_prompt.chars().count() + input_cursor.min(*input_len)
            } else {
                cursor.x as usize
            };

        // 跟随底部时按 screen 行取逐列样式对齐像素；否则退化纯文本。
        let following =
            following && visible_lines.len() as u16 == self.component.read(cx).grid.rows();
        let mut i = 0usize;
        for line_text in &visible_lines {
            let top = bounds.origin.y + px(i as f32 * f32::from(prepaint.line_h));
            if display_prompt != native_prompt {
                if let Some((line, _, _)) =
                    normalize_prompt_line(line_text, &native_prompt, &display_prompt)
                {
                    paint_prompt_line(&line, bounds, left, top, prepaint, &colors, window, cx);
                    i += 1;
                    continue;
                }
            }
            if following {
                let view = self.component.read(cx).grid.row_view(i);
                paint_term_row(&view, line_text, left, top, prepaint, &colors, window, cx);
            } else {
                paint_plain_line(line_text, left, top, prepaint, &colors, window, cx);
            }
            i += 1;
        }

        // 参数只作为淡色幽灵文本绘制，不写入 grid，因此不会改变实际光标位置。
        let inline_hint = if following {
            self.component.read(cx).inline_completion_hint()
        } else {
            None
        };
        if let Some(hint) = inline_hint.as_deref() {
            let row_top = bounds.origin.y + px(cursor.y as f32 * f32::from(prepaint.line_h));
            paint_completion_hint(
                hint,
                display_cursor_x,
                left,
                row_top,
                prepaint,
                &colors,
                window,
                cx,
            );
        }

        // 选区高亮（跟随底部时按屏幕坐标铺半透明底）。
        if following && self.component.read(cx).grid.has_selection() {
            paint_selection(&self.component, bounds, prepaint, &colors, window, cx);
        }

        // 光标块（仅跟随底部时绘制）。
        if following {
            let row_top = bounds.origin.y + px(cursor.y as f32 * f32::from(prepaint.line_h));
            let x_px = bounds.origin.x + px(display_cursor_x as f32 * f32::from(prepaint.cell_w));
            // 细竖线光标在空格、中文和幽灵提示前都能明确显示位置。
            let caret = Bounds::new(point(x_px, row_top), size(px(1.5), prepaint.line_h));
            // 记录光标矩形（window 坐标），供模型层 `bounds_for_range` 返回，让输入法候选栏
            // 跟随光标显示（否则 macOS 候选栏落到屏幕左下角）。
            self.component
                .update(cx, |comp, _| comp.ime_caret_bounds = Some(caret));
            window.paint_quad(fill(caret, colors.default_fg));
        }

        // 状态栏：展示 adapter 提供的会话标题 + 状态文案（真正的 adapter 信息，而非占位）。
        let (session_title, session_status) = {
            let comp = self.component.read(cx);
            let status = comp.adapter.status_text(comp.state);
            let status = comp
                .failure_reason
                .as_deref()
                .map(|reason| format!("{status}: {reason}"))
                .unwrap_or(status);
            (comp.adapter.title(), status)
        };
        paint_status_bar(
            bounds,
            prepaint,
            &session_title,
            &session_status,
            &colors,
            window,
            cx,
        );

        // 危险命令确认条。
        let pending = self.component.read(cx).pending_dangerous.clone();
        if let Some(dangerous) = pending {
            paint_danger_bar(bounds, prepaint, &dangerous, &colors, window, cx);
        }
    }
}

fn paint_term_row(
    view: &TermRowView,
    line: &str,
    left: f32,
    top: Pixels,
    layout: &TerminalLayout,
    colors: &TermColors,
    window: &mut Window,
    cx: &mut App,
) {
    // 先画默认前景整行，再对逐列彩色段覆盖。
    paint_plain_line(line, left, top, layout, colors, window, cx);
    let mut i = 0usize;
    while i < view.chars.len() {
        // 找到一个连续同前景色区间。
        let fg = view.fg[i];
        let mut j = i + 1;
        while j < view.chars.len() && view.fg[j] == fg {
            j += 1;
        }
        if let Some((r, g, b)) = fg {
            let seg: String = view.chars[i..j].iter().collect();
            // 逐列 (u8,u8,u8) → 打包 0xRRGGBB，供 gpui `rgb(hex: u32)` 使用。
            let packed = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
            paint_color_run(&seg, i, rgb(packed), left, top, layout, window, cx);
        }
        i = j;
    }
}

/// 绘制产品统一的 Redis CLI 当前输入行，遮掉后端原生的 host:port 提示符。
fn paint_prompt_line(
    line: &str,
    bounds: Bounds<Pixels>,
    left: f32,
    top: Pixels,
    layout: &TerminalLayout,
    colors: &TermColors,
    window: &mut Window,
    cx: &mut App,
) {
    let row = Bounds::new(
        point(bounds.origin.x, top),
        size(bounds.size.width, layout.line_h),
    );
    window.paint_quad(fill(row, colors.bg));
    paint_plain_line(line, left, top, layout, colors, window, cx);
}

/// 把 PTY 中的后端原生提示符转换为产品统一提示符。
/// 返回值同时保留原生前缀长度和输入长度，供光标位置换算。
fn normalize_prompt_line(
    line: &str,
    native_prompt: &str,
    display_prompt: &str,
) -> Option<(String, usize, usize)> {
    let native_prefix = native_prompt.trim_end();
    // 光标位置换算用的"原生前缀单元数"必须含提示符尾部空格：
    // `suffix` 剥掉的是 trim_end 后的前缀 + 1 个原生空格，而 `cursor.x` 从含尾部空格的提示符算起，
    // 若这里取 trim_end 长度，空输入时光标会被多推 1 格，提示符与光标之间出现多余边距。
    let native_prefix_len = native_prompt.chars().count();
    let suffix = line.strip_prefix(native_prefix)?;
    let input = suffix.strip_prefix(' ').unwrap_or(suffix);
    Some((
        format!("{display_prompt}{input}"),
        native_prefix_len,
        // 输入长度按"单元宽"累计（宽字符=2），否则光标 x 撞上宽字符时会被错误夹短。
        input.chars().map(|c| char_cell_width(c) as usize).sum(),
    ))
}

fn paint_color_run(
    text: &str,
    col: usize,
    color: gpui::Rgba,
    left: f32,
    top: Pixels,
    layout: &TerminalLayout,
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() {
        return;
    }
    // 按单元宽度分段绘制，覆盖默认前景色段的着色；宽字符第 2 格(空白)不绘制。
    let chars: Vec<char> = text.chars().collect();
    paint_cell_runs(&chars, col, color, left, top, layout, window, cx);
}

fn paint_plain_line(
    line: &str,
    left: f32,
    top: Pixels,
    layout: &TerminalLayout,
    colors: &TermColors,
    window: &mut Window,
    cx: &mut App,
) {
    let chars: Vec<char> = line.chars().collect();
    paint_cell_runs(
        &chars,
        0,
        colors.default_fg.into(),
        left,
        top,
        layout,
        window,
        cx,
    );
}

/// 按终端单元宽度把字符序列切成"半角 / 全角"连续段，逐段成形并画在对应列。
/// - 半角段用基准字号，前进 1 格/字符；全角段把字形缩放为 2 格宽并定位在起始列，
///   这样字形恰好占 2 格，光标(按格计数)相对字形不会漂移，且字间无空格空隙。
fn paint_cell_runs(
    chars: &[char],
    start_col: usize,
    color: gpui::Rgba,
    left: f32,
    top: Pixels,
    layout: &TerminalLayout,
    window: &mut Window,
    cx: &mut App,
) {
    let mut col = start_col;
    let mut i = 0usize;
    while i < chars.len() {
        let w = char_cell_width(chars[i]) as usize;
        if w > 1 {
            // 全角段：连续收集全角字符；每个占 2 格，步进跳过其第 2 个空白格。
            let mut seg = String::new();
            let mut cells = 0usize;
            while i < chars.len() {
                let cw = char_cell_width(chars[i]) as usize;
                if cw <= 1 {
                    break;
                }
                seg.push(chars[i]);
                cells += cw;
                i += cw;
            }
            paint_run(
                &seg,
                col,
                TERMINAL_FONT_SIZE * layout.cjk_scale,
                color,
                left,
                top,
                layout,
                window,
                cx,
            );
            col += cells;
        } else {
            // 半角段（含真实空格，如输入的空格与行尾填充格）。
            let mut j = i;
            let mut cells = 0usize;
            while j < chars.len() && char_cell_width(chars[j]) as usize == 1 {
                j += 1;
                cells += 1;
            }
            paint_run(
                &chars[i..j].iter().collect::<String>(),
                col,
                TERMINAL_FONT_SIZE,
                color,
                left,
                top,
                layout,
                window,
                cx,
            );
            col += cells;
            i = j;
        }
    }
}

/// 用指定字号把一段文本成形并画在 (left + col*cell_w, top)。
fn paint_run(
    text: &str,
    col: usize,
    font_size: f32,
    color: gpui::Rgba,
    left: f32,
    top: Pixels,
    layout: &TerminalLayout,
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() {
        return;
    }
    let run = TextRun {
        len: text.len(), // TextRun.len 为字节数（gpui 用其做 utf8 切片），不能按 chars 数。
        font: terminal_font(),
        color: color.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(
        SharedString::from(text.to_string()),
        font_size.into(),
        &[run],
        None,
    );
    let x = px(left + col as f32 * f32::from(layout.cell_w));
    let _ = shaped.paint(point(x, top), layout.line_h, TextAlign::Left, None, window, cx);
}

fn paint_completion_hint(
    hint: &str,
    col: usize,
    left: f32,
    top: Pixels,
    layout: &TerminalLayout,
    colors: &TermColors,
    window: &mut Window,
    cx: &mut App,
) {
    if hint.is_empty() {
        return;
    }
    let run = TextRun {
        len: hint.len(),
        font: terminal_font(),
        color: colors.completion_hint_fg.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(
        SharedString::from(hint.to_string()),
        (TERMINAL_FONT_SIZE as f32).into(),
        &[run],
        None,
    );
    let x = px(left + col as f32 * f32::from(layout.cell_w));
    let _ = shaped.paint(
        point(x, top),
        layout.line_h,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

fn paint_selection(
    component: &Entity<TerminalComponent>,
    bounds: Bounds<Pixels>,
    layout: &TerminalLayout,
    colors: &TermColors,
    window: &mut Window,
    cx: &mut App,
) {
    // 逐行找出连续选中的列区间，绘一块半透明 bg（文本已先画好，铺在其上的高亮接近选区观感）。
    let comp = component.read(cx);
    for y in 0..comp.grid.rows() as usize {
        let mut x = 0usize;
        while x < comp.grid.cols() as usize {
            if !comp.grid.is_cell_selected(x, y) {
                x += 1;
                continue;
            }
            let start = x;
            while x < comp.grid.cols() as usize && comp.grid.is_cell_selected(x, y) {
                x += 1;
            }
            let cell = Bounds::new(
                point(
                    bounds.origin.x + px(start as f32 * f32::from(layout.cell_w)),
                    bounds.origin.y + px(y as f32 * f32::from(layout.line_h)),
                ),
                size(
                    px((x - start) as f32 * f32::from(layout.cell_w)),
                    layout.line_h,
                ),
            );
            window.paint_quad(fill(cell, colors.selection_bg));
        }
    }
}

fn paint_status_bar(
    bounds: Bounds<Pixels>,
    layout: &TerminalLayout,
    title: &str,
    status: &str,
    colors: &TermColors,
    window: &mut Window,
    cx: &mut App,
) {
    let status_top = bounds.origin.y + bounds.size.height - layout.status_h;
    let bar = Bounds::new(
        point(bounds.origin.x, status_top),
        size(bounds.size.width, layout.status_h),
    );
    window.paint_quad(fill(bar, colors.status_bg));

    // 标题（adapter.title）加粗靠左，状态文案（adapter.status_text）跟随其后。
    // 用整段单 run 绘制，宽度受限于状态栏，超长由上层 clip。
    let text = format!("  {title}   ·   {status}");
    let run = TextRun {
        len: text.len(), // TextRun.len 为字节数（gpui 用其做 utf8 切片），不能按 chars 数。
        font: terminal_font(),
        color: colors.status_fg.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(
        SharedString::from(text),
        (TERMINAL_FONT_SIZE as f32).into(),
        &[run],
        None,
    );
    let _ = shaped.paint(
        point(bounds.origin.x + px(8.0), status_top + px(4.0)),
        layout.line_h,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

fn paint_danger_bar(
    bounds: Bounds<Pixels>,
    layout: &TerminalLayout,
    dangerous: &str,
    colors: &TermColors,
    window: &mut Window,
    cx: &mut App,
) {
    let h = px(28.0);
    let top = bounds.origin.y + bounds.size.height - layout.status_h - h;
    let bar = Bounds::new(point(bounds.origin.x, top), size(bounds.size.width, h));
    window.paint_quad(fill(bar, colors.danger_bg));

    let text = format!("ⓘ 危险命令待确认: {dangerous}    Enter 确认 · Esc 取消");
    let run = TextRun {
        len: text.len(), // TextRun.len 为字节数（gpui 用其做 utf8 切片），不能按 chars 数。
        font: terminal_font(),
        color: colors.danger_fg.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(
        SharedString::from(text),
        (TERMINAL_FONT_SIZE as f32).into(),
        &[run],
        None,
    );
    let _ = shaped.paint(
        point(bounds.origin.x + px(8.0), top + px(5.0)),
        layout.line_h,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

/// 测一个全角字符（用中文字形 `的`）在基准字号下的前进宽度，用于计算全角缩放比。
fn measure_cjk_cell(window: &Window, _cx: &App) -> Pixels {
    let run = TextRun {
        len: 3, // `的` 为 3 字节 utf8
        font: terminal_font(),
        color: Hsla::white(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(
        SharedString::from("的"),
        (TERMINAL_FONT_SIZE as f32).into(),
        &[run],
        None,
    );
    line.width
}

fn measure_terminal_cell(window: &Window, _cx: &App) -> Pixels {
    let run = TextRun {
        len: 1,
        font: terminal_font(),
        color: Hsla::white(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(
        SharedString::from("0"),
        (TERMINAL_FONT_SIZE as f32).into(),
        &[run],
        None,
    );
    px(f32::from(line.width).max(7.0))
}

fn terminal_font() -> gpui::Font {
    gpui::font(TERMINAL_FONT)
}
