// 函数签名提示跟随光标：光标移出未闭合的调用括号后，浮层必须消失。
//
// 用真实 `NavicatMain` + 真实编辑器，事件全部来自模拟键入与点击；
// 回归点：鼠标点击过去不刷新签名，旧的 `COALESCE(expr, ...)` 会一直停在屏幕上。

#[gpui::test]
fn signature_tooltip_disappears_after_mouse_moves_cursor_outside_call(cx: &mut TestAppContext) {
    use std::time::{Duration, Instant};

    use fluxdb_core::ConnectionId;

    let (view, tabs, cx) =
        open_completion_view(cx, vec![oracle_completion_tab(ConnectionId(93))]);
    let editor = focus_editor(cx, &view, tabs[0]);

    // 光标停在未闭合的 `COALESCE(` 里，键入后应出现签名提示。
    cx.simulate_input("SELECT COALESCE(1");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        cx.background_executor
            .advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
        let signature = cx.update(|_, cx| editor.read(cx).signature.clone());
        if let Some(signature) = signature {
            assert!(
                signature.label.to_ascii_uppercase().contains("COALESCE"),
                "签名应指向 COALESCE，实际：{}",
                signature.label
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "键入后未出现函数签名提示"
        );
        std::thread::yield_now();
    }

    // 点击首行行首：光标离开调用括号，签名提示必须消失。
    let bounds = cx.debug_bounds("query-editor").expect("SQL editor");
    cx.simulate_click(
        gpui::point(bounds.left() + px(4.), bounds.top() + px(4.)),
        Modifiers::default(),
    );
    cx.background_executor
        .advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    let signature = cx.update(|_, cx| editor.read(cx).signature.clone());
    assert!(
        signature.is_none(),
        "光标移出调用后签名提示仍在：{}",
        signature.expect("check").label
    );
}
