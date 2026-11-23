// 长 CJK 行横向滚动回归（issue #159）。
//
// 逐字键入一行含中文的长 SQL 后，内容宽度必须继续增长；否则软换行关闭时横向
// 滚动条不出现、End 键/拖动也到不了行尾。覆盖两处根因：最长行宽度提示在编辑后
// 未重新测量，以及 CJK 字符被按 1 个展示列计算导致宽度低估。

#[gpui::test]
fn typing_long_cjk_line_grows_horizontal_scroll_extent(cx: &mut TestAppContext) {
    use fluxdb_core::ConnectionId;

    let (view, tabs, cx) = open_completion_view(cx, vec![oracle_completion_tab(ConnectionId(159))]);
    let editor = focus_editor(cx, &view, tabs[0]);

    // 先输入短文本，让最长行提示以较小宽度建立，再逐字追加长 CJK 行。
    cx.simulate_input("SELECT 1");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    cx.simulate_input(concat!(
        "update app_auth set app_key='修复任务运行会话顶栏编辑时，右侧内容被裁掉且无法横向滚动到末尾",
        "修复任务运行会话顶栏编辑时，右侧内容被裁掉且无法横向滚动到末尾",
        "修复任务运行会话顶栏编辑时，右侧内容被裁掉且无法横向滚动到末尾",
        "修复任务运行会话顶栏编辑时，右侧内容被裁掉且无法横向滚动到末尾'",
    ));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });

    let (soft_wrap, content_width, viewport_width, max_offset_x) = cx.update(|window, cx| {
        let editor = editor.read(cx);
        (
            editor.soft_wrap,
            f32::from(editor.content_width(window)),
            f32::from(editor.scroll_handle.bounds().size.width),
            f32::from(editor.scroll_handle.max_offset().x),
        )
    });
    assert!(
        !soft_wrap,
        "回归前提：默认软换行关闭，走横向滚动路径"
    );
    assert!(
        content_width > viewport_width,
        "长 CJK 行应让内容宽于视口：content={content_width} viewport={viewport_width}"
    );
    assert!(
        max_offset_x > 0.,
        "长 CJK 行应产生横向滚动余量，实际 max_offset.x={max_offset_x}"
    );
}
