// json_component/render.rs —— JSON 组件渲染。
//
// 提供独立、不依赖 `NavicatMain` 状态的只读预览渲染，供 Workbench 等调用点复用：
// 复用 `json_editor` 的行拆分（`json_editor_rows`）、语法高亮（`redis_json_text_row`）
// 与主题（`JsonEditorTheme`），非法 JSON 时安全回退为原文，保证结果区始终可读。

/// 把组件渲染成展示元素（`component.rs` 的渲染入口）。
#[allow(dead_code)] // 完整渲染入口（含编辑态）；Workbench 轻量接入走 `render_preview`
fn json_component_view(
    component: &JsonComponent,
    colors: UiColors,
    _window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let theme = JsonEditorTheme::from_colors(colors);
    match component.mode {
        JsonMode::Preview => {
            let rows = json_editor_rows(component.state());
            // 只读预览：结构化行 + 语法高亮；非法 JSON 时 `json_editor_rows` 会回退为原文行。
            json_component_rows_block(&rows, theme, component.config.indent_size, colors)
        }
        JsonMode::Edit => {
            // 编辑态：复用组件内部持有的 InputState 全文编辑。Preview 之外的调用方应在
            // `begin_edit(window, cx)` 后进入此分支；此处直接绘制内部输入实体。
            json_component_edit_body(component, colors, cx)
        }
    }
}

/// 只读预览：渲染结构化 JSON 行（带语法高亮）。
///
/// 内容高度按其行数自然增长，由外层滚动容器（如 Workbench 结果卡片 body）负责纵向滚动；
/// 非法 JSON 时 `json_editor_rows` 回退为原文行，保证预览始终可读、不空白。
fn json_component_rows_block(
    rows: &[JsonViewRow],
    theme: JsonEditorTheme,
    indent_size: usize,
    colors: UiColors,
) -> Div {
    div()
        .size_full()
        .min_h(px(0.))
        .bg(colors.input_bg)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .children(rows.iter().map(|row| {
                    redis_json_text_row(row, theme, indent_size, true)
                })),
        )
}

/// 编辑态主体：绘制组件内部 `InputState`（组件自持，不向外暴露）。
#[allow(dead_code)] // 编辑态供未来消费方接入；当前仅 Preview 被 Workbench 使用
fn json_component_edit_body(
    component: &JsonComponent,
    colors: UiColors,
    _cx: &mut Context<NavicatMain>,
) -> Div {
    let theme = JsonEditorTheme::from_colors(colors);
    let diagnostic = component.diagnostic();
    div()
        .size_full()
        .bg(colors.input_bg)
        .overflow_hidden()
        .child({
            // 无窗口 / 未进入编辑态时的兜底，避免空白。
            let text = component.source();
            div()
                .size_full()
                .px_2()
                .py_2()
                .border_1()
                .border_color(
                    if diagnostic.is_some() { theme.error_border } else { colors.border },
                )
                .rounded(colors.radius)
                .font_family("Menlo")
                .text_size(px(13.))
                .text_color(colors.text)
                .child(text)
        })
}
