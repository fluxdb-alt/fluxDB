// 可选中文本元素与文本选择层的接线验证：真实元素 + 真实 `TextSelectionLayer`，无头驱动。
//
// 覆盖两条用户路径：双击选词（MongoDB 文档卡片）与跨行拖选、复制（JSON 展示）。

mod selectable_styled_text_tests {
    use gpui::{
        Context, InteractiveElement as _, IntoElement, Modifiers, MouseButton, MouseDownEvent,
        MouseMoveEvent, MouseUpEvent, ParentElement as _, Render, Styled as _, TestAppContext,
        VisualTestContext, Window, div, point, px,
    };
    use gpui_kit::base::TextSelection;

    use crate::{SelectableStyledText, apply_component_theme_colors, extend_component_i18n};

    struct SelectionView;

    impl Render for SelectionView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let row = |selector: &'static str, id: &'static str, text: &'static str, order: u64| {
                div()
                    .debug_selector(move || selector.to_string())
                    .font_family(crate::PLATFORM_MONOSPACE_FONT)
                    .text_size(px(13.))
                    .child(
                        SelectableStyledText::new(id, text)
                            .highlights(vec![(
                                0..text.len(),
                                gpui::HighlightStyle {
                                    color: Some(gpui::rgba(0xffffff).into()),
                                    ..Default::default()
                                },
                            )])
                            .document_order(order),
                    )
            };
            div()
                .size_full()
                .child(gpui_kit::base::TextSelectionLayer)
                .child(row("selection-row-0", "row-0", "alpha beta", 0))
                .child(row("selection-row-1", "row-1", "gamma delta", 1))
        }
    }

    fn init_theme(cx: &mut TestAppContext) {
        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
            apply_component_theme_colors(gpui_component::ThemeMode::Light, cx);
        });
    }

    fn mount(cx: &mut TestAppContext) -> &mut VisualTestContext {
        let (_, cx) = cx.add_window_view(|_, _| SelectionView);
        cx.simulate_resize(gpui::size(px(320.), px(200.)));
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx
    }

    fn row_bounds(cx: &mut VisualTestContext, selector: &'static str) -> gpui::Bounds<gpui::Pixels> {
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} 必须渲染"))
    }

    #[gpui::test]
    fn double_click_selects_a_word(cx: &mut TestAppContext) {
        init_theme(cx);
        let cx = mount(cx);
        let row = row_bounds(cx, "selection-row-0");
        // 行首第 1~5 个字符是 `alpha`：双击应只选中单词，而不是整行。
        let position = point(row.left() + px(6.), row.center().y);
        cx.simulate_event(MouseDownEvent {
            position,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 2,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            position,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 2,
        });
        // 选择在下一帧 paint 时才投影到参与者，断言前必须真正重绘一次。
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            assert_eq!(
                TextSelection::selected_text(window, cx),
                "alpha",
                "双击应选中光标所在单词"
            );
        });
    }

    #[gpui::test]
    fn drag_selects_across_rows_and_orders_by_document_order(cx: &mut TestAppContext) {
        init_theme(cx);
        let cx = mount(cx);
        let first = row_bounds(cx, "selection-row-0");
        let second = row_bounds(cx, "selection-row-1");
        // 从第一行行首拖到第二行行首：跨行按 document_order 拼接，行间以换行分隔。
        cx.simulate_event(MouseDownEvent {
            position: point(first.left() + px(2.), first.center().y),
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 1,
            first_mouse: false,
        });
        cx.simulate_event(MouseMoveEvent {
            position: point(second.left() + px(2.), second.center().y),
            modifiers: Modifiers::default(),
            pressed_button: Some(MouseButton::Left),
        });
        cx.simulate_event(MouseUpEvent {
            position: point(second.left() + px(2.), second.center().y),
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 1,
        });
        // 选择在下一帧 paint 时才投影到参与者，断言前必须真正重绘一次。
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            assert_eq!(TextSelection::selected_text(window, cx), "alpha beta");
        });
    }
}
