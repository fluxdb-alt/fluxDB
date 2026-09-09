// editor_component/render.rs —— 编辑器渲染：Render trait + 自定义 canvas 元素。
//
// 严格遵循 request_layout / prepaint / paint 三段分离；只渲染视口附近的行，
// 并把布局命中的交互区域回写到 Editor 状态供输入层使用。
//
// 绘制文案全部使用旧 sql_editor 的同款 idioma：shape_line(...).paint(...)，
// 颜色使用 gpui::rgb(...)，避免 gpui 2.x 中不存在的 paint_text / Rgba::from 接口。

use crate::{app_icon_path, AppIcon};

use gpui::CursorStyle;
use gpui_component::input::Input;

/// 方案 A：编辑后保持强制连续排帧的窗口。窗口内输入回显即时（~60Hz），
/// 超过后回到事件驱动（静止无脏区不重绘，避免空转耗电）。
const INPUT_REFRESH_WINDOW: Duration = Duration::from_millis(500);

fn paint_fps_metrics(
    frames: u32,
    elapsed: Duration,
    total_paint: Duration,
    max_paint: Duration,
) -> (f64, f64, f64) {
    let frames = frames.max(1) as f64;
    (
        frames / elapsed.as_secs_f64(),
        total_paint.as_secs_f64() * 1000.0 / frames,
        max_paint.as_secs_f64() * 1000.0,
    )
}

#[cfg(test)]
mod paint_perf_tests {
    use super::*;

    #[test]
    fn fps_metrics_use_seconds_and_milliseconds() {
        let (fps, avg_ms, max_ms) = paint_fps_metrics(
            60,
            Duration::from_secs(2),
            Duration::from_millis(120),
            Duration::from_millis(8),
        );
        assert!((fps - 30.0).abs() < f64::EPSILON);
        assert!((avg_ms - 2.0).abs() < f64::EPSILON);
        assert!((max_ms - 8.0).abs() < f64::EPSILON);
    }
}

/// 把编辑器动作绑定到编辑器根元素：GPUI 键绑定只派发给在此 `.on_action` 注册的元素。
/// 泛型 `A` 由 `_marker: fn() -> A` 标注具体动作类型，委托给 `Editor::dispatch_action`。
/// Div 的 `.on_action` 以 `&mut App` 为上下文，故此处通过 `Entity::update` 进入编辑器上下文。
/// （下沉自宿主 content_views.rs；编辑器自持全部键鼠交互，宿主不再重复脚手架。）
fn bind_editor_action<A: gpui::Action>(
    editor: gpui::Entity<Editor>,
    _marker: fn() -> A,
) -> impl Fn(&A, &mut Window, &mut gpui::App) + Clone + 'static {
    move |action: &A, _window: &mut Window, app: &mut gpui::App| {
        editor.update(app, |editor, cx| editor.dispatch_action(action, cx));
    }
}

// 键鼠/滚动/滚动条交互全部下沉到编辑器自身（对齐 Zed 的编辑器中滚动模型）。
// 宿主只保留布局定位与配色，见 content_views.rs 两个承载面板的瘦身。
impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = cx.entity();
        let find_open = self.find_open();
        let find_input = self.find_input.clone();
        let replace_input = self.replace_input.clone();
        let theme = self.theme;
        let element_id = editor.entity_id();
        if find_open {
            // `open_find` can be triggered by an action without a Window handle; sync the
            // selected text when the panel is rendered, where GPUI provides the handle.
            let query = self.find_state.query.clone();
            if find_input.read(cx).value().as_ref() != query.as_str() {
                find_input.update(cx, |input, cx| input.set_value(query, window, cx));
            }
        }
        // 补全浮层打开时，先对全量候选定型一次宽度并缓存，避免滚动到不同标签时逐帧
        // 重算导致抖动（见 completion_placement 对 completion_width 的读取）。
        if self.completion_width.is_none()
            && self.completion_visible
            && !self.completion_items.is_empty()
        {
            self.completion_width = Some(px(completion_popup_width_for_items(
                &self.completion_items,
                &self.editor_font(),
                px(self.font_size),
                window,
            )));
        }

        let focus_handle = self.focus_handle.clone();
        let scroll_handle = self.scroll_handle.clone();
        // 内容：保留编辑器实际高度（.relative + .flex_shrink_0），使 track_scroll
        // 容器能以内容尺寸计算可滚动范围（默认 stretch 会拉满视口，导致不可滚）。
        let scroll_content = div()
            .relative()
            .flex_shrink_0()
            .child(EditorCanvas { editor: editor.clone() });
        let mut root = div()
            .id(("fluxdb-editor", element_id))
            .relative()
            .size_full()
            .overflow_hidden()
            .key_context(CONTEXT)
            .track_focus(&focus_handle)
            .tab_index(0)
            .cursor(gpui::CursorStyle::IBeam)
            // 点击聚焦（先），再进编辑器自身点击命中（框选/折叠/运行/CodeLens）。
            .on_mouse_down(MouseButton::Left, {
                let focus_handle = focus_handle.clone();
                move |_, window, app| {
                    focus_handle.focus(window, app);
                }
            })
            .on_mouse_down(MouseButton::Left, {
                let editor = editor.clone();
                move |event, window, app| {
                    editor.update(app, |editor, cx| editor.mouse_down(event, window, cx));
                }
            })
            .on_mouse_up(MouseButton::Left, {
                let editor = editor.clone();
                move |event, window, app| {
                    editor.update(app, |editor, cx| editor.mouse_up(event, window, cx));
                }
            })
            .on_mouse_move({
                let editor = editor.clone();
                move |event, window, app| {
                    editor.update(app, |editor, cx| editor.mouse_move(event, window, cx));
                }
            })
            // 滚轮：逐事件把原始 delta 交给 scroll（scroll 内部按「连续手势」在
            // 动画目标上累加、「新手势」以当前 offset 为基准重算，并做主轴锁定）。
            // 不做跨事件 coalesce：那会累积放大位移（滚动过快），且让滚到顶后残留的
            // 纵向累积阻塞后续横向滚动（mac 触摸板小位移的方向切换）。
            .on_scroll_wheel({
                let editor = editor.clone();
                move |event, window, app| {
                    editor.update(app, |editor, cx| {
                        editor.scroll(event, event.delta, window, cx);
                    });
                }
            })
            // 动作分发：GPUI 的键绑定只派发给在此注册 .on_action 的元素。
            .on_action(bind_editor_action(editor.clone(), || Backspace))
            .on_action(bind_editor_action(editor.clone(), || Delete))
            .on_action(bind_editor_action(editor.clone(), || Enter { secondary: false }))
            .on_action(bind_editor_action(editor.clone(), || Escape))
            .on_action(bind_editor_action(editor.clone(), || MoveUp))
            .on_action(bind_editor_action(editor.clone(), || MoveDown))
            .on_action(bind_editor_action(editor.clone(), || MoveLeft))
            .on_action(bind_editor_action(editor.clone(), || MoveRight))
            .on_action(bind_editor_action(editor.clone(), || MoveHome))
            .on_action(bind_editor_action(editor.clone(), || MoveEnd))
            .on_action(bind_editor_action(editor.clone(), || MoveToStart))
            .on_action(bind_editor_action(editor.clone(), || MoveToEnd))
            .on_action(bind_editor_action(editor.clone(), || MoveToPreviousWord))
            .on_action(bind_editor_action(editor.clone(), || MoveToNextWord))
            .on_action(bind_editor_action(editor.clone(), || SelectAll))
            .on_action(bind_editor_action(editor.clone(), || SelectLeft))
            .on_action(bind_editor_action(editor.clone(), || SelectRight))
            .on_action(bind_editor_action(editor.clone(), || SelectUp))
            .on_action(bind_editor_action(editor.clone(), || SelectDown))
            .on_action(bind_editor_action(editor.clone(), || SelectHome))
            .on_action(bind_editor_action(editor.clone(), || SelectEnd))
            .on_action(bind_editor_action(editor.clone(), || SelectToStart))
            .on_action(bind_editor_action(editor.clone(), || SelectToEnd))
            .on_action(bind_editor_action(editor.clone(), || SelectToPreviousWord))
            .on_action(bind_editor_action(editor.clone(), || SelectToNextWord))
            .on_action(bind_editor_action(editor.clone(), || SelectLine))
            .on_action(bind_editor_action(editor.clone(), || Undo))
            .on_action(bind_editor_action(editor.clone(), || Redo))
            .on_action(bind_editor_action(editor.clone(), || Copy))
            .on_action(bind_editor_action(editor.clone(), || Cut))
            .on_action(bind_editor_action(editor.clone(), || Paste))
            .on_action(bind_editor_action(editor.clone(), || IndentInline))
            .on_action(bind_editor_action(editor.clone(), || OutdentInline))
            .on_action(bind_editor_action(editor.clone(), || ToggleLineComment))
            .on_action(bind_editor_action(editor.clone(), || ToggleFold))
            .on_action(bind_editor_action(editor.clone(), || FoldAll))
            .on_action(bind_editor_action(editor.clone(), || UnfoldAll))
            .on_action(bind_editor_action(editor.clone(), || TriggerCompletion))
            .on_action(bind_editor_action(editor.clone(), || OpenFind))
            .on_action(bind_editor_action(editor.clone(), || CloseFind))
            .on_action(bind_editor_action(editor.clone(), || FindNext))
            .on_action(bind_editor_action(editor.clone(), || FindPrevious))
            // 滚动视口容器：保留 items_start 防内容被拉伸；overflow_hidden 不放行
            // GPUI 内建滚轮（滚轮由上方 on_scroll_wheel 自处理，避免同一事件处理两次）。
            .child(
                div()
                    .absolute()
                    .id(("editor-scroll", element_id))
                    .inset_0()
                    .flex()
                    .items_start()
                    .size_full()
                    .track_scroll(&scroll_handle)
                    .overflow_hidden()
                    .child(scroll_content),
            )
            // 滚动条 overlay（统一显示：SQL 与 Redis 编辑器行为一致）。
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(Scrollbar::new(&scroll_handle)),
            );
        // 浮层数据在此从 `self` 提取（不借 render 的 `&mut Context` 读 Editor，避免
        // 重入 panic）；事件闭包再各自捕获 Editor Entity 做 accept / hover。
        let popup_data = self.completion_popup_data(window);
        if let Some(data) = popup_data {
            root = root.child(completion_popup(editor.clone(), data));
        }
        if find_open {
            root = root.child(find_bar(editor, find_input, replace_input, theme));
        }
        root
    }
}

fn find_bar(
    editor: gpui::Entity<Editor>,
    find_input: gpui::Entity<gpui_component::input::InputState>,
    replace_input: gpui::Entity<gpui_component::input::InputState>,
    theme: EditorTheme,
) -> impl IntoElement {
    let next_editor = editor.clone();
    let previous_editor = editor.clone();
    let replace_editor = editor.clone();
    let replace_all_editor = editor.clone();
    let close_editor = editor.clone();
    div()
        .absolute()
        .top(px(8.))
        .right(px(12.))
        .w(px(420.))
        .p_2()
        .gap_1()
        .flex()
        .flex_col()
        .bg(theme.completion_bg)
        .border_1()
        .border_color(theme.line_number)
        .shadow(vec![box_shadow(px(0.), px(8.), px(18.), px(0.), hsla(0., 0., 0., 0.22))])
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex()
                .gap_1()
                .child(Input::new(&find_input).small())
                .child(find_bar_button("上一个", theme, move |app| {
                    previous_editor.update(app, |editor, cx| editor.find_previous(cx));
                }))
                .child(find_bar_button("下一个", theme, move |app| {
                    next_editor.update(app, |editor, cx| editor.find_next(cx));
                }))
                .child(find_bar_button("关闭", theme, move |app| {
                    close_editor.update(app, |editor, cx| editor.close_find(cx));
                })),
        )
        .child(
            div()
                .flex()
                .gap_1()
                .child(Input::new(&replace_input).small())
                .child(find_bar_button("替换", theme, move |app| {
                    replace_editor.update(app, |editor, cx| editor.find_replace_current(cx));
                }))
                .child(find_bar_button("全部替换", theme, move |app| {
                    replace_all_editor.update(app, |editor, cx| editor.find_replace_all(cx));
                })),
        )
}

fn find_bar_button(
    label: &'static str,
    theme: EditorTheme,
    on_click: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    div()
        .h(px(26.))
        .px_2()
        .flex()
        .items_center()
        .cursor_pointer()
        .text_size(px(11.))
        .text_color(theme.completion_text)
        .bg(theme.active_line)
        .hover(|style| style.bg(theme.completion_selected_bg))
        .on_mouse_down(MouseButton::Left, move |_, _, app| on_click(app))
        .child(label)
}

pub(crate) struct EditorCanvas {
    editor: gpui::Entity<Editor>,
}

impl IntoElement for EditorCanvas {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorCanvas {
    type RequestLayoutState = ();
    type PrepaintState = LayoutSnapshot;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.editor.update(cx, |editor, _| {
            editor.update_wrap_width(window);
            editor.sync_inline_hints(window);
        });
        let editor = self.editor.read(cx);
        let mut style = gpui::Style::default();
        style.size.width = editor.content_width(window).into();
        style.size.height = editor.content_height(window).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        // request_layout 已在本帧执行过 update_wrap_width（软换行宽度变化时才重建），
        // 此处不再重复调用，滚动/纯重绘帧不触碰折行重建路径。
        let editor = self.editor.read(cx);
        editor.build_layout(window)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let started = Instant::now();
        let (
            focus_handle,
            scroll_handle,
            scroll_offset,
            fold_candidates,
            folded_rows,
            gutter_hovered,
            show_folding,
            mouse_position,
        ) = {
            let editor = self.editor.read(cx);
            let fold_candidates = editor
                .fold_candidates()
                .into_iter()
                .map(|(_, fold)| fold.start_row)
                .collect::<std::collections::BTreeSet<_>>();
            // 已折叠起始行来自稳定折叠状态（DM-106），按当前快照解析。
            let folded_rows = editor
                .active_folds()
                .into_iter()
                .map(|fold| fold.start_row)
                .collect::<std::collections::BTreeSet<_>>();
            (
                editor.focus_handle.clone(),
                editor.scroll_handle.clone(),
                editor.scroll_handle.offset(),
                fold_candidates,
                folded_rows,
                editor.gutter_hovered,
                editor.profile.show_folding,
                window.mouse_position(),
            )
        };

        // 整改 5.x：滚动动画进行时，在 paint/prepaint 阶段调用
        // `request_animation_frame` 强制窗口按 ~60Hz 持续排帧（gpui 官方动画走法，
        // 参考 elements/animation.rs）。此调用 notify 的是顶层 current_view（实时
        // invalidate_view），而非 Editor 子视图（后者走延迟 Effect::Notify 队列、
        // 处理时撞 draw_phase gate 被丢弃，正是 inv_skip 高的根因）。
        //
        // 方案 A：编辑后的短时间内同样强制连续排帧，消除输入回显延迟（静止时
        // 事件驱动帧稀疏会让回显最多滞后 ~100-300ms）。窗口过后回到静止
        // 无脏区不重绘，避免空转。仅读取 last_edit_at，不改状态。
        let input_refresh = self
            .editor
            .read(cx)
            .last_edit_at
            .elapsed()
            < INPUT_REFRESH_WINDOW;
        if self.editor.read(cx).scroll_animating || input_refresh {
            window.request_animation_frame();
        }

        // IME/文本输入：需要 Editor 实现 EntityInputHandler。
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );

        // 把布局阶段计算的命中/光标区域回写。
        self.editor.update(cx, |editor, _| {
            editor.line_hit_regions = prepaint.line_hit_regions.clone();
            editor.code_lens_hits.clear();
            editor.last_cursor_local_bounds = prepaint.cursor_local;
        });

        // 从编辑器当前注入的主题读取配色（整改设计 4.2：不再使用固定深色兜底作为唯一实现）。
        let colors = self.editor.read(cx).theme;
        let viewport = scroll_handle.bounds();
        window.paint_quad(gpui::fill(viewport, colors.background));
        // 行号列固定在视口左侧；正文横向滚动时必须裁到 gutter 右边，避免长行覆盖行号。
        let content_left = viewport.left()
            + px(EDITOR_PADDING_X)
            + prepaint.gutter_width
            + px(EDITOR_CONTENT_GAP);
        let content_mask = ContentMask {
            bounds: Bounds::new(
                GPoint::new(content_left, viewport.top()),
                gpui::Size::new(
                    (viewport.right() - content_left).max(px(0.)),
                    viewport.size.height,
                ),
            ),
        };

        // 语句执行状态行背景：按可视片段定位，兼容软换行/折叠。
        paint_line_backgrounds(&prepaint.line_backgrounds, prepaint, viewport, scroll_offset, window, &colors);

        // CodeLens：按目标行合并动作，使用 Zed 同款 ` | ` 分隔并放在代码行上方。
        let mut code_lens_hits = Vec::new();
        window.with_content_mask(Some(content_mask.clone()), |window| {
        for lens_line in &prepaint.code_lenses {
            let Some(line) = prepaint
                .visible_lines
                .iter()
                .find(|line| line.buffer_row == lens_line.row && line.first_fragment)
            else {
                continue;
            };
            let y = viewport.top()
                + scroll_offset.y
                + self
                    .editor
                    .read(cx)
                    .y_for_visual_row(line.visual_row, prepaint.line_height);
            let base_x = viewport.left()
                + scroll_offset.x
                + px(EDITOR_PADDING_X)
                + prepaint.gutter_width
                + px(EDITOR_CONTENT_GAP)
                + px(lens_line.indent_column as f32 * self.editor.read(cx).measure_character_width(window));
            let lens_y = y - px(CODE_LENS_HEIGHT);
            let lens_font = self.editor.read(cx).editor_font();
            let lens_size = prepaint.font_size * 0.9;
            let mut x = base_x;
            for (index, lens) in lens_line.lenses.iter().enumerate() {
                if index > 0 {
                    let separator = SharedString::from(" | ");
                    let run = TextRun {
                        len: separator.len(),
                        font: lens_font.clone(),
                        color: colors.line_number.into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let shaped = window.text_system().shape_line(separator, lens_size.into(), &[run], None);
                    shaped.paint(GPoint::new(x, lens_y), px(CODE_LENS_HEIGHT), TextAlign::Left, None, window, cx).ok();
                    x += shaped.width;
                }
                let title = SharedString::from(lens.title.clone());
                let run = TextRun {
                    len: title.len(),
                    font: lens_font.clone(),
                    color: colors.line_number.into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let shaped = window.text_system().shape_line(title, lens_size.into(), &[run], None);
                let lens_bounds = Bounds::new(
                    GPoint::new(x, lens_y),
                    gpui::Size::new(shaped.width, px(CODE_LENS_HEIGHT)),
                )
                .intersect(&content_mask.bounds);
                if lens_bounds.contains(&mouse_position) {
                    // CodeLens 是链接式动作：沿用 Zed/GPUI 的 pointing-hand 语义，
                    // 并用轻量背景提示当前可点击范围。
                    window.set_window_cursor_style(CursorStyle::PointingHand);
                    let hover_background = gpui::Rgba {
                        r: colors.line_number.r,
                        g: colors.line_number.g,
                        b: colors.line_number.b,
                        a: 0.18,
                    };
                    window.paint_quad(gpui::fill(lens_bounds, hover_background));
                }
                let hit_index = code_lens_hits.len();
                code_lens_hits.push(CodeLensHit {
                    range: lens.range,
                    action: lens.action.clone(),
                });
                prepaint.line_hit_regions.push(LineHitRegion {
                    row: lens_line.row,
                    bounds: lens_bounds,
                    kind: LineHitKind::CodeLens(hit_index),
                });
                shaped.paint(GPoint::new(x, lens_y), px(CODE_LENS_HEIGHT), TextAlign::Left, None, window, cx).ok();
                x += shaped.width;
            }
        }
        });
        self.editor.update(cx, |editor, _| {
            editor.code_lens_hits = code_lens_hits;
            editor.line_hit_regions = prepaint.line_hit_regions.clone();
        });

        let line_height = prepaint.line_height;
        for line in &prepaint.visible_lines {
            let visual_index = line.visual_row;
            let y = viewport.top()
                + scroll_offset.y
                + self
                    .editor
                    .read(cx)
                    .y_for_visual_row(visual_index, line_height);
            let line_origin = GPoint::new(
                viewport.left()
                    + scroll_offset.x
                    + px(EDITOR_PADDING_X)
                    + prepaint.gutter_width
                    + px(EDITOR_CONTENT_GAP),
                y,
            );

            if line.current_line {
                let active = Bounds::new(
                    GPoint::new(viewport.left(), y),
                    gpui::Size::new(viewport.size.width, line_height),
                );
                window.paint_quad(gpui::fill(active, colors.active_line));
            }

            if line.first_fragment
                && show_folding
                && fold_candidates.contains(&line.buffer_row)
                && (line.current_line
                    || gutter_hovered
                    || folded_rows.contains(&line.buffer_row))
            {
                let icon = if folded_rows.contains(&line.buffer_row) {
                    AppIcon::ChevronRight
                } else {
                    AppIcon::ChevronDown
                };
                // 折叠箭头属于 gutter，固定不随正文横向滚动（与行号列对齐）。
                let icon_bounds = Bounds::new(
                    GPoint::new(
                        viewport.left()
                            + px(EDITOR_PADDING_X)
                            + prepaint.gutter_width
                            - px(EDITOR_FOLD_GUTTER)
                            + px(3.),
                        y + (line_height - px(12.)) * 0.5,
                    ),
                    gpui::Size::new(px(12.), px(12.)),
                );
                window
                    .paint_svg(
                        icon_bounds,
                        SharedString::from(app_icon_path(icon)),
                        Default::default(),
                        Default::default(),
                        colors.line_number.into(),
                        cx,
                    )
                    .ok();
            }

            // 行号
            if line.first_fragment {
                let number = SharedString::from((line.buffer_row + 1).to_string());
                let number_run = TextRun {
                    len: number.len(),
                    font: self.editor.read(cx).editor_font(),
                    color: colors.line_number.into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let number_line = window
                    .text_system()
                    .shape_line(number, prepaint.font_size().into(), &[number_run], None);
                // 行号固定在 gutter 列，去掉横向滚动偏移（正文才随 scroll_offset.x 滚动）。
                let number_origin = GPoint::new(
                    viewport.left()
                        + px(EDITOR_PADDING_X)
                        + px(EDITOR_CONTENT_GAP),
                    line_origin.y,
                );
                number_line
                    .paint(number_origin, line_height, TextAlign::Left, None, window, cx)
                    .ok();
            }

            // 空字节区间（空 buffer 行）无需 shaping；其余行一律走 shape_visual_line，
            // 其内部按 shaped 缓存命中与否决定是否提取文本。
            let shaped = if line.byte_range.start == line.byte_range.end {
                None
            } else {
                Some(self.editor.read(cx).shape_visual_line(line, window))
            };

            window.with_content_mask(Some(content_mask.clone()), |window| {
                // 选区背景必须先于文本绘制，并按当前软换行片段裁剪，避免覆盖文字或整行铺色。
                if let Some(sel) = &prepaint.selected_range {
                    let start = sel.start.max(line.byte_range.start);
                    let end = sel.end.min(line.byte_range.end);
                    if end > start {
                        let fragment_start = line.byte_range.start;
                        let start_x = shaped
                            .as_ref()
                            .map(|line| line.x_for_index(start - fragment_start))
                            .unwrap_or_default();
                        let end_x = shaped
                            .as_ref()
                            .map(|line| line.x_for_index(end - fragment_start))
                            .unwrap_or(start_x);
                        let bounds = Bounds::new(
                            GPoint::new(line_origin.x + start_x, y),
                            gpui::Size::new((end_x - start_x).max(px(1.)), line_height),
                        );
                        window.paint_quad(gpui::fill(bounds, colors.selection));
                    }
                }

                // 文本（带选区/语法/IME 高亮）。
                if let Some(shaped) = shaped {
                    shaped.paint(line_origin, line_height, TextAlign::Left, None, window, cx).ok();
                }
            });

        }

        // 光标
        if let Some(cursor_bounds) = prepaint.cursor_local {
            if self.editor.read(cx).cursor_visible && focus_handle.is_focused(window) {
                window.with_content_mask(Some(content_mask), |window| {
                    window.paint_quad(gpui::fill(cursor_bounds, colors.cursor));
                });
            }
        }

        // 补全浮层已 Element 化（render() 中作为兄弟节点挂载，见 completion_popup.rs）。

        // 函数签名提示浮层（P1.9）：非阻塞 tooltip，绘制在光标下方。
        if let Some(cursor_bounds) = prepaint.cursor_local {
            paint_signature_tooltip(&self.editor, cursor_bounds, window, cx);
        }
        let paint_elapsed = started.elapsed();
        let fps_report = self.editor.update(cx, |editor, _| {
            let now = Instant::now();
            editor.perf_frame_count += 1;
            editor.perf_paint_total += paint_elapsed;
            editor.perf_paint_max = editor.perf_paint_max.max(paint_elapsed);
            // GPUI 不直接暴露 vsync 丢帧计数；按 16ms frame budget 估算被阻塞的帧数。
            let frame_ms = paint_elapsed.as_secs_f64() * 1_000.0;
            let missed = (frame_ms / 16.0).floor();
            if missed > 1.0 {
                editor.perf_dropped_frames = editor
                    .perf_dropped_frames
                    .saturating_add((missed - 1.0) as u64);
            }

            let elapsed = now.duration_since(editor.perf_window_started);
            if elapsed < Duration::from_secs(1) {
                return None;
            }

            let frames = editor.perf_frame_count;
            let (fps, avg_paint_ms, max_paint_ms) =
                paint_fps_metrics(frames, elapsed, editor.perf_paint_total, editor.perf_paint_max);
            editor.perf_window_started = now;
            editor.perf_frame_count = 0;
            editor.perf_paint_total = Duration::ZERO;
            editor.perf_paint_max = Duration::ZERO;
            let dropped_frames = editor.perf_dropped_frames;
            editor.perf_dropped_frames = 0;
            Some((fps, avg_paint_ms, max_paint_ms, frames, dropped_frames))
        });
        if let Some((fps, avg_paint_ms, max_paint_ms, frames, dropped_frames)) = fps_report {
            let editor = self.editor.read(cx);
            let in_flight_tasks = [
                editor._completion_task.is_some(),
                editor._hover_task.is_some(),
                editor._diagnostics_task.is_some(),
                editor._syntax_task.is_some(),
                editor._signature_task.is_some(),
            ]
            .into_iter()
            .filter(|active| *active)
            .count();
            tracing::info!(
                target: "gdb_editor_perf",
                op = "fps",
                editor_id = editor.perf_editor_id,
                edit_id = editor.perf_edit_id,
                fps,
                frames,
                avg_paint_ms,
                max_paint_ms,
                text_bytes = editor.buffer.len(),
                line_count = editor.buffer.line_count(),
                visible_rows = prepaint.visible_lines.len(),
                buffer_version = editor.buffer.version(),
                request_id = 0u64,
                task_id = 0u64,
                layer = "display",
                in_flight_tasks,
                dropped_frames,
            );
        }
        let paint_elapsed_us = paint_elapsed.as_micros() as u64;
        tracing::debug!(
            target: "gdb_editor_perf",
            op = "paint",
            editor_id = self.editor.read(cx).perf_editor_id,
            edit_id = self.editor.read(cx).perf_edit_id,
            elapsed_us = paint_elapsed_us,
            visible_rows = prepaint.visible_lines.len(),
            code_lens = prepaint.code_lenses.len(),
            buffer_version = self.editor.read(cx).buffer.version(),
            request_id = 0u64,
            task_id = 0u64,
            layer = "display",
        );
        if paint_elapsed_us > fluxdb_editor_core::FRAME_BUDGET_US {
            tracing::warn!(
                target: "gdb_editor_perf",
                op = "paint_slow",
                editor_id = self.editor.read(cx).perf_editor_id,
                edit_id = self.editor.read(cx).perf_edit_id,
                elapsed_us = paint_elapsed_us,
                visible_rows = prepaint.visible_lines.len(),
                code_lens = prepaint.code_lenses.len(),
                buffer_version = self.editor.read(cx).buffer.version(),
                request_id = 0u64,
                task_id = 0u64,
                layer = "display",
            );
        }
    }
}

/// 布局快照的便捷访问。
impl LayoutSnapshot {
    pub(crate) fn font_size(&self) -> f32 {
        self.font_size
    }
}

/// 绘制语句执行状态的行背景装饰。
///
    /// `line_backgrounds` 为 (buffer_row, 状态键)，每个可见片段按自身 visual row 绘制。
/// 状态键只取通用执行语义（running / success / failure），具体的 SQL 状态映射由
/// 接入层（SQL 适配器）负责，通用编辑器不做 SQL 语义判断。
fn paint_line_backgrounds(
    line_backgrounds: &[(usize, String)],
    prepaint: &LayoutSnapshot,
    viewport: Bounds<Pixels>,
    scroll_offset: gpui::Point<Pixels>,
    window: &mut Window,
    colors: &EditorTheme,
) {
    if line_backgrounds.is_empty() {
        return;
    }
    for (row, key) in line_backgrounds {
        let Some(color) = line_background_color(key, colors) else {
            continue;
        };
        for (i, line) in prepaint.visible_lines.iter().enumerate() {
            if line.buffer_row != *row {
                continue;
            }
            let y = viewport.top()
                + scroll_offset.y
                + prepaint.line_y[i];
            let bounds = Bounds::new(
                GPoint::new(viewport.left(), y),
                gpui::Size::new(viewport.size.width, prepaint.line_height),
            );
            window.paint_quad(gpui::fill(bounds, color));
        }
    }
}

/// 状态键 -> 行背景颜色。只认识通用执行状态键，未知键返回 `None`（不绘制）。
fn line_background_color(key: &str, colors: &EditorTheme) -> Option<gpui::Rgba> {
    match key {
        "running" => Some(colors.status_running),
        "success" => Some(colors.status_success),
        "failure" => Some(colors.status_failure),
        _ => None,
    }
}

/// 编辑器文本颜色（供 paint 使用）。
impl Editor {
    pub(crate) fn shape_visual_line(
        &self,
        line: &VisualLineRender,
        window: &Window,
    ) -> gpui::ShapedLine {
        let started = Instant::now();
        let key = (
            self.buffer.version(),
            line.byte_range.start,
            line.byte_range.end,
            self.font_size.to_bits(),
        );
        if let Some(shaped) = self.shaped_line_cache.borrow().get(&key).cloned() {
            tracing::debug!(
                target: "gdb_editor_perf",
                op = "line_shaping",
                cache_hit = true,
                elapsed_us = started.elapsed().as_micros() as u64,
                buffer_version = self.buffer.version(),
                byte_start = line.byte_range.start,
                byte_end = line.byte_range.end,
                request_id = 0u64,
                task_id = 0u64,
                layer = "layout",
            );
            return shaped;
        }
        // 仅缓存未命中（首次进视口 / 内容变更）才提取文本并 shaping；滚动帧命中缓存
        // 不会走到这里，避免每帧按行重复分配字符串。字节区间端点防御性裁剪到当前长度。
        let len = self.buffer.len();
        let text = self.buffer.text_in_range(CoreRange::new(
            line.byte_range.start.min(len),
            line.byte_range.end.min(len),
        ));
        let runs = self.text_runs_for_line(line, &self.theme, &text);
        let shaped = window.text_system().shape_line(
            SharedString::from(text),
            self.font_size.into(),
            &runs,
            None,
        );
        let mut cache = self.shaped_line_cache.borrow_mut();
        // Keep the cache bounded while allowing scrolls to reuse recently painted lines.
        if cache.len() >= 2048 {
            cache.clear();
        }
        cache.insert(key, shaped.clone());
        tracing::debug!(
            target: "gdb_editor_perf",
            op = "line_shaping",
            cache_hit = false,
            elapsed_us = started.elapsed().as_micros() as u64,
            buffer_version = self.buffer.version(),
            byte_start = line.byte_range.start,
            byte_end = line.byte_range.end,
            request_id = 0u64,
            task_id = 0u64,
            layer = "layout",
        );
        shaped
    }

    /// 为单行构造文本 run 列表，按语法高亮分段着色。
    ///
    /// 语法高亮优先从分块索引取与 `line.byte_range` 相交的子集
    /// （可见行局部读取，不遍历整文档），映射 token 颜色；高亮之间的间隙回落默认文本色。
    /// `TextRun.len` 为 utf8 字节数，与 GPUI 约定一致（见 gpui::TextRun::len）。
    /// `text` 由调用方在 shaped 缓存未命中时才提取，供本行语法 run 分段使用。
    fn text_runs_for_line(
        &self,
        line: &VisualLineRender,
        colors: &EditorTheme,
        text: &str,
    ) -> Vec<TextRun> {
        if text.is_empty() {
            return Vec::new();
        }
        let line_start = line.byte_range.start;
        let line_end = line.byte_range.end;
        let default_color: gpui::Hsla = colors.text.into();

        // 收集与本行相交的高亮，裁剪为行内相对字节区间 (s, e)。
        let mut syntax_spans: Vec<(usize, usize, gpui::Hsla)> = Vec::new();
        if let Some(syntax) = &self.syntax {
            let query_started = Instant::now();
            // DM-404：渲染只走单一 HighlightStore 的 viewport 相交查询（量随可见行 token，
            // 不随全文）。高亮按 buffer 字节区间裁剪；fold/inlay 坐标由 DisplaySnapshot 输出端
            // 负责（DM-407），此处无需感知折叠/嵌入层。
            for h in syntax
                .highlight_store
                .iter_intersecting(CoreRange::new(line_start, line_end))
            {
                if h.range.end <= line_start || h.range.start >= line_end {
                    continue;
                }
                let Some(color) = colors.syntax_color(&h.kind) else {
                    continue;
                };
                let s = h.range.start.max(line_start) - line_start;
                let e = (h.range.end.min(line_end) - line_start).min(text.len());
                if e > s {
                    syntax_spans.push((s, e, color.into()));
                }
            }
            tracing::debug!(
                target: "gdb_editor_perf",
                op = "highlight_viewport_query",
                editor_id = self.perf_editor_id,
                edit_id = self.perf_edit_id,
                buffer_version = self.buffer.version(),
                request_id = 0u64,
                task_id = 0u64,
                layer = "highlight",
                elapsed_us = query_started.elapsed().as_micros() as u64,
                byte_start = line_start,
                byte_end = line_end,
                spans = syntax_spans.len(),
            );
        } else if let Some(provider) = &self.providers.syntax {
            // 完整解析在后台进行时，先用语言适配器的轻量词法器填充可见行。
            // shape_visual_line 会缓存结果，因此不会在每帧重复扫描同一行。
            for h in provider.highlight_visible(&self.buffer.snapshot(), line.byte_range) {
                if h.range.end <= line_start || h.range.start >= line_end {
                    continue;
                }
                let Some(color) = colors.syntax_color(&h.kind) else {
                    continue;
                };
                let s = h.range.start.max(line_start) - line_start;
                let e = (h.range.end.min(line_end) - line_start).min(text.len());
                if e > s {
                    syntax_spans.push((s, e, color.into()));
                }
            }
        }
        syntax_spans.sort_by_key(|(start, _, _)| *start);

        let diagnostic_items: Vec<&Diagnostic> = self
            .diagnostic_line_index
            .get(line.buffer_row)
            .into_iter()
            .flatten()
            .filter_map(|&index| self.diagnostics.get(index))
            .filter(|diag| diag.range.start < line_end && diag.range.end > line_start)
            .collect();

        let mut boundaries = vec![0, text.len()];
        boundaries.extend(syntax_spans.iter().flat_map(|(s, e, _)| [*s, *e]));
        boundaries.extend(diagnostic_items.iter().filter_map(|diag| {
            let start = diag.range.start.max(line_start).min(line_end) - line_start;
            let end = diag.range.end.max(line_start).min(line_end) - line_start;
            (end > start).then_some([start, end])
        }).flatten());
        // CJK 等多字节编辑下，`preserve_syntax_after_edit` 的字节平移可能让高亮区间端点
        // 落在字符中间；GPUI shape 按 TextRun 的字节 len 对文本切片时会在非字符边界 panic
        // （"byte index is not a char boundary"）。渲染前统一把边界吸附到 `text` 字符边界，
        // 覆盖 syntax store / 轻量词法 / diagnostic 三条来源，避免逐处补丁。
        boundaries = boundaries
            .into_iter()
            .map(|b| {
                let mut c = b.min(text.len());
                while c > 0 && !text.is_char_boundary(c) {
                    c -= 1;
                }
                c
            })
            .collect();
        boundaries.sort_unstable();
        boundaries.dedup();

        let mut runs: Vec<TextRun> = Vec::new();
        for window in boundaries.windows(2) {
            let [start, end] = [window[0], window[1]];
            if start == end {
                continue;
            }
            let color = syntax_spans
                .iter()
                // 后声明的查询捕获更具体，例如函数名也会命中通用对象名规则。
                .rfind(|(s, e, _)| *s <= start && start < *e)
                .map(|(_, _, color)| *color)
                .unwrap_or(default_color);
            let diagnostic = diagnostic_items
                .iter()
                .copied()
                .filter(|diag| {
                    diag.range.start <= line_start + start && line_start + start < diag.range.end
                })
                .max_by_key(|diag| diag.severity);
            let underline = diagnostic.map(|diag| gpui::UnderlineStyle {
                color: Some(match diag.severity {
                    fluxdb_editor_core::DiagnosticSeverity::Error => colors.error.into(),
                    fluxdb_editor_core::DiagnosticSeverity::Warning => colors.warning.into(),
                    fluxdb_editor_core::DiagnosticSeverity::Information
                    | fluxdb_editor_core::DiagnosticSeverity::Hint => colors.warning.into(),
                }),
                thickness: px(1.),
                wavy: true,
            });
            runs.push(self.editor_text_run(end - start, color, underline));
        }
        runs
    }

    /// 构造一个普通文本 run（UTF-8 字节长度的着色片段）。
    fn editor_text_run(
        &self,
        len: usize,
        color: gpui::Hsla,
        underline: Option<gpui::UnderlineStyle>,
    ) -> TextRun {
        TextRun {
            len,
            font: self.editor_font(),
            color,
            background_color: None,
            underline,
            strikethrough: None,
        }
    }
}

/// 绘制函数签名提示浮层（P1.9）：在光标下方绘制非阻塞的参数 tooltip。
///
/// 仅绘制面板与 label，不捕获输入（不阻塞输入）；当前参数使用主题高亮背景标出。
fn paint_signature_tooltip(
    editor: &gpui::Entity<Editor>,
    cursor_bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let (signature, colors, font, font_size) = {
        let editor = editor.read(cx);
        let Some(signature) = editor.signature.clone() else {
            return;
        };
        (signature, editor.theme, editor.editor_font(), editor.font_size)
    };
    let runs = signature_runs(
        &signature.label,
        signature.active_parameter,
        &signature.parameter_ranges,
        &colors,
        &font,
    );
    let pad_x = px(10.);
    let pad_y = px(6.);
    // 文本行高随字号，避免固定高度导致标签文字溢出提示框（顶部/底部被裁切）。
    let text_line_height = px(font_size + 6.);
    let width_avail = (px(420.) - pad_x * 2.).max(px(60.));
    // 超出宽度上限时截断到省略号，避免长 label（如深 schema 限定）画出框右边界。
    let (text, runs) = window
        .text_system()
        .line_wrapper(font.clone(), font_size.into())
        .truncate_line(
            SharedString::from(signature.label.clone()),
            width_avail,
            "…",
            &runs,
            TruncateFrom::End,
        );
    let shaped = window
        .text_system()
        .shape_line(text, font_size.into(), &runs, None);
    let width = (shaped.width + pad_x * 2.).clamp(px(60.), px(420.));
    let height = text_line_height + pad_y * 2.;
    // 放在光标下方，越界时上移，避免被视口底部裁切。
    let mut top = cursor_bounds.bottom() + px(4.);
    if top + height > window.bounds().bottom() {
        top = (cursor_bounds.top() - height - px(4.)).max(window.bounds().top() + px(4.));
    }
    let left = cursor_bounds
        .left()
        .min((window.bounds().right() - width - px(8.)).max(window.bounds().left() + px(4.)));
    let bounds = Bounds::new(GPoint::new(left, top), Size::new(width, height));
    window.paint_quad(
        gpui::fill(bounds, colors.completion_bg)
            .corner_radii(gpui_component::Theme::global(cx).radius_lg)
            .border_widths(px(1.))
            .border_color(colors.line_number),
    );
    shaped
        .paint(
            GPoint::new(left + pad_x, top + pad_y),
            text_line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        )
        .ok();
}

fn signature_runs(
    label: &str,
    active_parameter: Option<usize>,
    parameter_ranges: &[(usize, usize)],
    colors: &EditorTheme,
    font: &gpui::Font,
) -> Vec<TextRun> {
    let base = colors.completion_text.into();

    // 显式参数范围（Redis 等空格/方括号骨架）：直接按范围切分并高亮 active 参数。
    if !parameter_ranges.is_empty() {
        let param_len = parameter_ranges.len();
        let active = active_parameter.unwrap_or(0).min(param_len.saturating_sub(1));
        let mut runs = Vec::new();
        let mut cursor = 0;
        for (index, &(start, end)) in parameter_ranges.iter().enumerate() {
            if start > cursor {
                runs.push(text_run(start - cursor, base, font));
            }
            let mut run = text_run(end.saturating_sub(start), base, font);
            if index == active && end > start {
                run.background_color = Some(colors.completion_highlight.into());
            }
            if run.len > 0 {
                runs.push(run);
            }
            // 参数之间（含空格）用默认色。
            cursor = end;
        }
        if cursor < label.len() {
            runs.push(text_run(label.len() - cursor, base, font));
        }
        return runs;
    }

    // 默认规则（SQL 函数签名）：按 `()` + 逗号解析 label。
    let Some(open) = label.find('(') else {
        return vec![text_run(label.len(), base, font)];
    };
    let Some(close) = label.rfind(')') else {
        return vec![text_run(label.len(), base, font)];
    };
    let mut params = Vec::new();
    let mut start = open + 1;
    let mut depth = 0usize;
    for (offset, ch) in label[open + 1..close].char_indices() {
        let index = open + 1 + offset;
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                params.push((start, index));
                start = index + 1;
            }
            _ => {}
        }
    }
    params.push((start, close));

    let mut runs = Vec::new();
    let mut cursor = 0;
    for (index, (param_start, param_end)) in params.iter().enumerate() {
        if *param_start > cursor {
            runs.push(text_run(*param_start - cursor, base, font));
        }
        let mut run = text_run(param_end.saturating_sub(*param_start), base, font);
        if active_parameter == Some(index) && *param_end > *param_start {
            run.background_color = Some(colors.completion_highlight.into());
        }
        if run.len > 0 {
            runs.push(run);
        }
        cursor = *param_end;
    }
    if cursor < label.len() {
        runs.push(text_run(label.len() - cursor, base, font));
    }
    runs
}

/// 构造 label 的着色 run 列表：查询词命中的子串用高亮色，其余用默认文本色。
///
/// 返回 (展示文本, runs)。展示文本可能被前导/尾随空白裁剪；runs 的 len 为 UTF-8 字节数，
/// 与 GPUI `TextRun::len` 约定一致。
fn highlight_label_runs(
    label: &str,
    query: &str,
    colors: &EditorTheme,
    font: &gpui::Font,
) -> (String, Vec<TextRun>) {
    let base: gpui::Hsla = colors.completion_text.into();
    let highlight: gpui::Hsla = colors.completion_highlight.into();
    let mut runs: Vec<TextRun> = Vec::new();
    if query.is_empty() || label.is_empty() {
        runs.push(text_run(label.len(), base, font));
        return (label.to_string(), runs);
    }
    let needle = query.to_lowercase();
    let hay = label.to_lowercase();
    let mut from = 0usize;
    let mut pos = 0usize;
    while let Some(rel) = hay[from..].find(&needle) {
        let match_start = from + rel;
        if match_start > pos {
            runs.push(text_run(match_start - pos, base, font));
        }
        runs.push(text_run(needle.len(), highlight, font));
        pos = match_start + needle.len();
        from = pos;
        if from >= hay.len() {
            break;
        }
    }
    if pos < label.len() {
        runs.push(text_run(label.len() - pos, base, font));
    }
    (label.to_string(), runs)
}

fn text_run(len: usize, color: gpui::Hsla, font: &gpui::Font) -> TextRun {
    TextRun {
        len,
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

fn completion_kind_icon(kind: fluxdb_editor_core::CompletionKind) -> AppIcon {
    match kind {
        fluxdb_editor_core::CompletionKind::Keyword => AppIcon::Text,
        fluxdb_editor_core::CompletionKind::Function => AppIcon::FileSql,
        fluxdb_editor_core::CompletionKind::Table => AppIcon::Table,
        fluxdb_editor_core::CompletionKind::Column => AppIcon::List,
        fluxdb_editor_core::CompletionKind::Schema => AppIcon::Database,
        fluxdb_editor_core::CompletionKind::Method => AppIcon::FileSql,
        _ => AppIcon::Select,
    }
}

#[cfg(test)]
mod render_syntax_tests {
    use super::*;

    /// 构造仅含语法 token 颜色的 EditorTheme（其余字段无关，置零）。
    fn colors() -> EditorTheme {
        EditorTheme {
            syntax_keyword: gpui::rgb(0x569cd6),
            syntax_string: gpui::rgb(0xd87979),
            syntax_number: gpui::rgb(0xb58cff),
            syntax_comment: gpui::rgb(0x7f9f7f),
            syntax_type: gpui::rgb(0x569cd6),
            syntax_identifier: gpui::rgb(0xffb52e),
            syntax_field: gpui::rgb(0x9cdcfe),
            syntax_function: gpui::rgb(0xdcdcaa),
            syntax_attribute: gpui::rgb(0xc586c0),
            syntax_variable: gpui::rgb(0x4ec9b0),
            syntax_parameter: gpui::rgb(0x9cdcfe),
            syntax_boolean: gpui::rgb(0x569cd6),
            // 非语法字段本测试不关心，统一置零。
            background: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            text: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            active_line: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            selection: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            cursor: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            line_number: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            error: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            warning: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            completion_bg: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            completion_selected_bg: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            completion_text: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            completion_detail: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            completion_highlight: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            status_running: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            status_success: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
            status_failure: gpui::Rgba { r: 0., g: 0., b: 0., a: 0. },
        }
    }

    #[test]
    fn syntax_color_maps_all_token_kinds_and_ignores_unknown() {
        let c = colors();
        for (kind, want) in [
            ("keyword", 0x569cd6),
            ("string", 0xd87979),
            ("number", 0xb58cff),
            ("comment", 0x7f9f7f),
            ("type", 0x569cd6),
            ("identifier", 0xffb52e),
            ("field", 0x9cdcfe),
            ("function", 0xdcdcaa),
            ("attribute", 0xc586c0),
            ("variable", 0x4ec9b0),
            ("parameter", 0x9cdcfe),
            ("boolean", 0x569cd6),
        ] {
            let got = c.syntax_color(kind).expect("known kind should map");
            assert_eq!(
                ((got.r * 255.).round() as u32) << 16
                    | ((got.g * 255.).round() as u32) << 8
                    | ((got.b * 255.).round() as u32),
                want,
                "kind {kind}"
            );
        }
        // 未知 kind 返回 None，由调用方回退默认文本色。
        assert!(c.syntax_color("marginalia").is_none());
        assert!(c.syntax_color("unknown_kind").is_none());
    }

    #[test]
    fn completion_kind_icons_use_one_small_monochrome_set() {
        assert_eq!(
            completion_kind_icon(fluxdb_editor_core::CompletionKind::Keyword),
            AppIcon::Text
        );
        assert_eq!(
            completion_kind_icon(fluxdb_editor_core::CompletionKind::Function),
            AppIcon::FileSql
        );
        assert_eq!(
            completion_kind_icon(fluxdb_editor_core::CompletionKind::Table),
            AppIcon::Table
        );
        assert_eq!(
            completion_kind_icon(fluxdb_editor_core::CompletionKind::Column),
            AppIcon::List
        );
        assert_eq!(
            completion_kind_icon(fluxdb_editor_core::CompletionKind::Schema),
            AppIcon::Database
        );
        assert_eq!(
            completion_kind_icon(fluxdb_editor_core::CompletionKind::Method),
            AppIcon::FileSql
        );
    }
}
