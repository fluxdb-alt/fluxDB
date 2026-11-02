// 可选中文本元素：在 GPUI 文本选择体系里渲染带语法高亮的单行文本。
//
// `gpui_base::SelectableText` 只接受纯文本 + 单一文本样式，无法保留 JSON token 配色，
// 因此按同一套 `TextSelection` 协议实现一个带 `StyledText` 高亮的等价元素。
// 使用前提：窗口根节点挂载了 `TextSelectionLayer`；未挂载时它只负责画文本。
//
// 本文件经 `include!` 落在 crate 根，与 foundation.rs 共享导入作用域，
// 所以 gpui 类型统一带前缀，避免与根作用域已有导入重名。

use gpui_kit::base::{TextSelectionHandle, TextSelectionRegistration, TextSelectionRun};

/// 带高亮的可选中文本；同一选择文档内靠 `document_order` 排序。
///
/// `text` 必须是 `styled_text` 的源文本，选择区间按它的字节偏移回投影。
pub(crate) struct SelectableStyledText {
    id: gpui::ElementId,
    text: gpui::SharedString,
    styled_text: gpui::StyledText,
    document_order: u64,
    copy_text: Option<gpui::SharedString>,
}

impl SelectableStyledText {
    pub(crate) fn new(id: impl Into<gpui::ElementId>, text: impl Into<gpui::SharedString>) -> Self {
        let text = text.into();
        Self {
            id: id.into(),
            styled_text: gpui::StyledText::new(text.clone()),
            text,
            document_order: 0,
            copy_text: None,
        }
    }

    /// 语法高亮区间必须落在 `text` 的字节边界上，升序且不重叠。
    pub(crate) fn highlights(
        mut self,
        highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    ) -> Self {
        self.styled_text = gpui::StyledText::new(self.text.clone()).with_highlights(highlights);
        self
    }

    /// 所在选择文档中的阅读顺序：跨行拖选与复制按它排。
    pub(crate) fn document_order(mut self, document_order: u64) -> Self {
        self.document_order = document_order;
        self
    }

    /// 选中后的复制内容：用于屏上是占位、真实内容在展开态的行（如折叠的 JSON 节点）。
    ///
    /// 给出后，只要该行参与选区，复制就输出这份完整内容，而不是选区里那几个字符。
    pub(crate) fn copy_text(mut self, copy_text: impl Into<gpui::SharedString>) -> Self {
        self.copy_text = Some(copy_text.into());
        self
    }

    fn paint_selection(
        layout: &gpui::TextLayout,
        range: Range<usize>,
        color: gpui::Hsla,
        window: &mut gpui::Window,
    ) {
        let (Some(start), Some(end)) = (
            layout.position_for_index(range.start),
            layout.position_for_index(range.end),
        ) else {
            return;
        };
        for bounds in selection_quad_bounds(start, end, layout.bounds(), layout.line_height()) {
            window.paint_quad(gpui::PaintQuad {
                bounds,
                background: color.into(),
                corner_radii: gpui::Corners::default(),
                border_widths: gpui::Edges::default(),
                border_color: gpui::transparent_black(),
                border_style: gpui::BorderStyle::default(),
            });
        }
    }
}

/// 与 gpui-base 同算法：首行到末尾、整行区间、末行到终点，支持换行后的多段高亮。
fn selection_quad_bounds(
    start: gpui::Point<gpui::Pixels>,
    end: gpui::Point<gpui::Pixels>,
    bounds: gpui::Bounds<gpui::Pixels>,
    line_height: gpui::Pixels,
) -> Vec<gpui::Bounds<gpui::Pixels>> {
    if start.y == end.y {
        return vec![gpui::Bounds::from_corners(
            start,
            gpui::Point::new(end.x, end.y + line_height),
        )];
    }

    let mut quads = vec![gpui::Bounds::from_corners(
        start,
        gpui::Point::new(bounds.right(), start.y + line_height),
    )];
    if end.y > start.y + line_height {
        quads.push(gpui::Bounds::from_corners(
            gpui::Point::new(bounds.left(), start.y + line_height),
            gpui::Point::new(bounds.right(), end.y),
        ));
    }
    quads.push(gpui::Bounds::from_corners(
        gpui::Point::new(bounds.left(), end.y),
        gpui::Point::new(end.x, end.y + line_height),
    ));
    quads
}

impl gpui::IntoElement for SelectableStyledText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// 跨帧保留的选择状态。
///
/// 订阅必须随元素状态一起存活：元素被替换或丢弃时退订，避免反复重建元素时泄漏订阅。
struct SelectionHandleState {
    handle: TextSelectionHandle,
    _refresh: gpui::Subscription,
}

impl gpui::Element for SelectableStyledText {
    type RequestLayoutState = TextSelectionHandle;
    type PrepaintState = gpui::Hitbox;

    fn id(&self) -> Option<gpui::ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        // 选择句柄按元素 id 保留，跨帧稳定，避免每帧重建选择状态。
        let handle = window.with_element_state(
            global_id.expect("SelectableStyledText 必须有稳定的元素 id"),
            |retained: Option<SelectionHandleState>, window| {
                let state = retained.unwrap_or_else(|| {
                    let handle = TextSelectionHandle::new(self.text.clone(), cx);
                    // 拖选期间 gpui 不会自动重绘：`dispatch_mouse_event` 只在原生拖放
                    // （`has_active_drag`）或悬停命中变化时 `refresh()`，文本拖选两者都
                    // 不满足。于是选区数据每次鼠标移动都更新了，高亮却要等下一次无关
                    // 重绘才画出来，看起来就是“按住拖了半天颜色才出现”。订阅选区变化
                    // 主动刷新窗口，每次指针更新立刻出一帧。
                    let refresh = handle.refresh_window_on_change(window, cx);
                    SelectionHandleState {
                        handle,
                        _refresh: refresh,
                    }
                });
                (state.handle.clone(), state)
            },
        );
        let (layout_id, ()) = self
            .styled_text
            .request_layout(global_id, inspector_id, window, cx);
        (layout_id, handle)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        handle: &mut Self::RequestLayoutState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        self.styled_text
            .prepaint(global_id, inspector_id, bounds, &mut (), window, cx);
        if let Some(copy_text) = &self.copy_text {
            let copy_text = copy_text.clone();
            handle.copy_with(move |_| copy_text.to_string(), cx);
        }
        let hitbox = window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal);
        // 选择层按命中框的内容遮罩判断“指针是否拖出了可视区”，进而自动滚动。
        // 字段行外面有 `overflow_hidden`（截断省略号），遮罩只剩一行高（约 22px），
        // 而自动滚动的触发区是边缘内 16px——几乎整行都算触发区，指针轻微一动就被
        // 当成拖出可视区，列表开始滚动，选区随内容一路跳到别的文档。
        // 这里把仅用于选择的遮罩放宽到整个视口，保留省略号裁剪，只关掉这次误触滚动。
        // （命中框本身仍用 `insert_hitbox` 注册，悬停判定不受影响。）
        let mut selection_hitbox = hitbox.clone();
        selection_hitbox.content_mask = gpui::ContentMask {
            bounds: gpui::Bounds::new(
                gpui::point(gpui::px(0.), gpui::px(0.)),
                window.viewport_size(),
            ),
        };
        let registration = TextSelectionRegistration::new(selection_hitbox, bounds)
            .with_document_order(self.document_order)
            .with_text_bounds(vec![bounds])
            .with_rendered_element(handle, window, cx);
        handle.register(registration, window, cx);
        hitbox
    }

    fn paint(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        handle: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        let layout = self.styled_text.layout().clone();
        // 不要每帧调 `TextSelection::selected_text` 判断选区是否变化：它要拼接整个
        // 窗口的选区文本，文档页几十上百行时是 O(N²)，拖动刷新会追不上鼠标。
        // 重绘由 `refresh_window_on_change` 订阅驱动，这里只算出本行的投影区间。
        let projection = handle.update_runs(
            &[
                TextSelectionRun::new(self.text.clone(), layout.clone(), bounds)
                    .with_document_order(self.document_order),
            ],
            cx,
        );
        let color = gpui_kit::base::Theme::global(cx).tokens.colors.selection;
        for range in projection.ranges().iter().flatten().cloned() {
            Self::paint_selection(&layout, range, color, window);
        }
        self.styled_text
            .paint(global_id, inspector_id, bounds, &mut (), &mut (), window, cx);
    }
}
