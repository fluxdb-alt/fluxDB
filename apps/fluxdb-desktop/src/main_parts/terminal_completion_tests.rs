// 终端补全浮层「候选超出可视高度必须能滚」的回归用例。
//
// 用假 adapter 直接渲染真实 `TerminalComponent`，不连库、不拉 PTY：这里验证的是
// 浮层自身的布局与滚动，与后端是哪一种 shell 无关。
//
// 历史缺陷：候选列表套了 `overflow_y_scrollbar()`（`Scrollable` 封装），它会丢掉
// 调用方传进来的 `track_scroll` 句柄、换成自己的内部句柄，于是 `scroll_to_item`
// 全部失效——方向键越过可见区后浮层纹丝不动；它画的滚动条还是随设置自动隐藏的，
// 候选被裁掉时静态画面上等于「没有滚动条」。
mod terminal_completion_tests {
    use fluxdb_core::terminal::{
        CompletionApplyEffect, TerminalCommandContext, TerminalCompletionAdapter,
        TerminalCompletionItem, TerminalCompletionKind, TerminalCompletionPriority,
        TerminalSessionAdapter, TerminalSessionKey, TerminalSessionKind, TerminalSessionState,
        TerminalSpawnSpec,
    };
    use gpui::{
        AppContext as _, Modifiers, ParentElement as _, Styled as _, TestAppContext,
        VisualTestContext,
    };

    use crate::{
        TERMINAL_CONTEXT, TerminalComponent, app_assets_base_path, extend_component_i18n, init_i18n,
        register_terminal_shortcuts,
    };

    /// 候选条数：20 条 × 30px 行高 = 608px，远超浮层 260px 的上限，必然溢出。
    const ITEM_COUNT: usize = 20;

    struct FakeSession;

    impl TerminalSessionAdapter for FakeSession {
        fn kind(&self) -> TerminalSessionKind {
            TerminalSessionKind::GenericShell
        }
        fn session_key(&self) -> TerminalSessionKey {
            TerminalSessionKey::GenericShell {
                profile_id: "fake".into(),
                working_dir: String::new(),
            }
        }
        fn title(&self) -> String {
            "fake".into()
        }
        fn status_text(&self, _state: TerminalSessionState) -> String {
            String::new()
        }
        fn spawn(&self) -> TerminalSpawnSpec {
            TerminalSpawnSpec {
                program: String::new(),
                args: Vec::new(),
                env: Vec::new(),
                cwd: None,
                cols: 80,
                rows: 24,
            }
        }
        fn on_output(&mut self, _bytes: &[u8], _state: &mut TerminalSessionState) {}
    }

    struct FakeCompletion;

    impl TerminalCompletionAdapter for FakeCompletion {
        fn supports_completion(&self) -> bool {
            true
        }
        fn complete(&self, _ctx: &TerminalCommandContext) -> Vec<TerminalCompletionItem> {
            (0..ITEM_COUNT)
                .map(|index| TerminalCompletionItem {
                    label: format!("item{index}"),
                    insert_text: format!("item{index}"),
                    replace_start: 0,
                    replace_end: 0,
                    detail: None,
                    kind: TerminalCompletionKind::Keyword,
                    priority: TerminalCompletionPriority::Normal,
                })
                .collect()
        }
        fn apply(&self, item: &TerminalCompletionItem) -> CompletionApplyEffect {
            CompletionApplyEffect::Insert {
                text: item.insert_text.clone(),
            }
        }
        fn is_dangerous(&self, _input: &str) -> bool {
            false
        }
    }

    struct Harness {
        terminal: gpui::Entity<TerminalComponent>,
    }

    impl gpui::Render for Harness {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::div().size_full().child(self.terminal.clone())
        }
    }

    fn launch(cx: &mut TestAppContext) -> (&mut VisualTestContext, gpui::Entity<Harness>) {
        let temp = tempfile::tempdir().expect("isolated profile");
        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "en");
        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
            register_terminal_shortcuts(cx);
        });
        let (harness, cx) = cx.add_window_view(|_window, cx| {
            let terminal = cx.new(|cx| {
                TerminalComponent::new(Box::new(FakeSession), Box::new(FakeCompletion), cx)
            });
            Harness { terminal }
        });
        cx.simulate_resize(gpui::size(gpui::px(1200.), gpui::px(800.)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (cx, harness)
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }

    /// 候选溢出时浮层必须能滚：键盘选到可视区之外，选中行要被滚进浮层内。
    #[gpui::test]
    fn terminal_completion_popup_scrolls_selection_into_view(cx: &mut TestAppContext) {
        let (cx, _harness) = launch(cx);
        let surface = cx
            .debug_bounds("terminal-surface")
            .expect("终端表面必须渲染");
        cx.simulate_click(surface.center(), Modifiers::default());
        draw(cx);

        // Tab 打开浮层（假 adapter 不边打字边提示）。
        cx.simulate_keystrokes("tab");
        draw(cx);
        let popup = cx
            .debug_bounds("terminal-completion-popup")
            .expect("补全浮层必须渲染");
        // 高度封顶在 260px：20 条候选若整块摊开就是 600px 以上。
        let popup_h = f32::from(popup.size.height);
        assert!(
            (200.0..=261.0).contains(&popup_h),
            "候选溢出时浮层高度应封顶（约 260px），实际 {popup_h}px：{popup:?}"
        );
        // 溢出确实存在：最后一条候选在浮层下方被裁掉。
        let last = cx
            .debug_bounds(leak(format!("terminal-completion-row:{}", ITEM_COUNT - 1)))
            .expect("最后一条候选必须参与布局");
        assert!(
            last.bottom() > popup.bottom(),
            "候选未溢出，用例前提不成立：最后一行 {last:?} 浮层 {popup:?}"
        );

        // ↓ 选到第 15 条：它必须被滚进浮层内，否则用户看不见自己选了什么。
        for _ in 0..15 {
            cx.simulate_keystrokes("down");
        }
        draw(cx);
        let popup = cx
            .debug_bounds("terminal-completion-popup")
            .expect("补全浮层必须仍在");
        let selected = cx
            .debug_bounds(leak("terminal-completion-row:15".to_string()))
            .expect("选中候选行必须参与布局");
        assert!(
            selected.top() >= popup.top() && selected.bottom() <= popup.bottom(),
            "选中行必须被滚进浮层（浮层丢弃 track_scroll 句柄时这里会失败）：行 {selected:?} 浮层 {popup:?}"
        );
        // 反向证明真的滚了：第一条已被推到浮层上方。
        let first = cx
            .debug_bounds(leak("terminal-completion-row:0".to_string()))
            .expect("第一条候选必须参与布局");
        assert!(
            first.top() < popup.top(),
            "滚动后第一条候选应移出浮层上方：行 {first:?} 浮层 {popup:?}"
        );
    }

    /// `debug_bounds` 只接受 `&'static str`；用例里按需把格式化后的选择器泄漏成静态串。
    fn leak(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }

    /// 终端 keymap 上下文常量必须与组件挂的 key_context 一致，否则按键根本到不了组件。
    #[test]
    fn terminal_context_is_wired() {
        assert_eq!(TERMINAL_CONTEXT, "Terminal");
    }
}
