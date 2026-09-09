// editor_component/completion_popup.rs —— 补全浮层 Element。
//
// 浮层已从 paint 手绘改为真实 Element 弹层：行列表由原生 ScrollHandle 驱动的滚动
// 容器承载（平滑逐像素滚轮 + 真实滚动条 + scroll_to_item 保持选中可见），对齐数据
// 表格的原生滚动手感。位置来自 completion_placement（编辑器根本地，viewport 相对）。
//
// 注意：本模块在 render() 中构建，不能借 render 的 `&mut Context` 读取 Editor
// （会触发 “cannot read while already being updated” 重入 panic）。所有数据由
// render() 侧经 completion_row / completion_popup 的普通参数传入；只有行/浮层的
// 事件闭包才捕获 Editor Entity —— 它们运行在事件 context（独立借用）下读取/更新，
// 安全。

/// 补全浮层可见数据的快照，由 render() 从 Editor 提取后传入。
pub(crate) struct CompletionPopupData {
    pub(crate) origin: GPoint,
    pub(crate) size: Size<Pixels>,
    pub(crate) theme: EditorTheme,
    pub(crate) font: gpui::Font,
    pub(crate) font_size: gpui::Pixels,
    pub(crate) query: String,
    pub(crate) selected: usize,
    pub(crate) scroll_handle: gpui::ScrollHandle,
    pub(crate) items: Vec<CompletionItem>,
    pub(crate) loading: bool,
    pub(crate) ambiguous: BTreeSet<String>,
    /// F005：选中项右侧 metadata 详情异步状态（None=无详情 provider 或浮层刚打开）。
    pub(crate) doc_state: Option<DocumentationState>,
}

/// 构造补全浮层（视为编辑器根的绝对定位兄弟节点，盖在其上）。
pub(crate) fn completion_popup(
    editor: gpui::Entity<Editor>,
    data: CompletionPopupData,
) -> impl IntoElement {
    let CompletionPopupData {
        origin,
        size,
        theme,
        font,
        font_size,
        query,
        selected,
        scroll_handle,
        items,
        loading,
        ambiguous,
        doc_state,
    } = data;

    // 滚动容器须显式限高（= 可见区高度，外框扣除 4*2 padding + 1*2 border = 10），
    // 否则随全量候选撑高、超出浮层边框；限高后内容超出即触发原生滚动。
    let scroll_w = size.width - px(10.);
    let scroll_h = size.height - px(10.);
    let selected_item = items.get(selected).cloned();
    // 详情面板属于弹窗内容宽度的一部分，左侧候选列不能继续使用整块宽度。
    let list_w = if doc_state.is_some() {
        scroll_w - px(COMPLETION_DOC_WIDTH + COMPLETION_DOC_DIVIDER)
    } else {
        scroll_w
    };
    let scroll_content = div()
        .id(ElementId::from("completion-popup-scroll")) // 需先带 .id() 转成 Stateful 才能调用滚动方法
        .w(list_w)
        .h(scroll_h)
        .overflow_y_scroll()
        .scrollbar_width(px(9.))
        .track_scroll(&scroll_handle)
        .flex()
        .flex_col()
        .children(items.into_iter().enumerate().map(|(index, item)| {
            completion_row(
                index,
                item,
                editor.clone(),
                selected,
                &ambiguous,
                theme,
                &font,
                font_size,
                &query,
            )
        }));

    let panel = div()
        .absolute()
            .left(origin.x)
            .top(origin.y)
            .w(size.width)
            .h(size.height)
            .p(px(4.))
            .bg(theme.completion_bg)
            .border_1()
            .border_color(theme.line_number)
            .shadow(vec![box_shadow(px(0.), px(8.), px(18.), px(0.), hsla(0., 0., 0., 0.22))])
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // 吞掉滚轮：滚在行上先被内层原生滚动器消费，冒泡到此被 stop；滚在 padding
            // 上直接在此被吞。避免滚轮上传到宿主面板把编辑器正文也滚走。
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(if loading {
                // 加载态占位：候选未就绪时显示单行「加载中…」。
                div().w_full().h_full().child(
                    div()
                        .id(ElementId::from("completion-popup-loading"))
                        .w(scroll_w)
                        .flex()
                        .items_center()
                        .px_2()
                        .h(px(COMPLETION_ROW_HEIGHT))
                        .text_size(font_size)
                        .child(
                            StyledText::new("加载中…").with_runs(vec![text_run(
                                "加载中…".len(),
                                theme.completion_detail.into(),
                                &font,
                            )]),
                        ),
                )
            } else {
                doc_content(
                    scroll_content,
                    doc_state,
                    selected_item.as_ref(),
                    theme,
                    &font,
                    font_size,
                    scroll_h,
                )
            });
    panel
}

/// F005：浮层内容。无详情时仅候选列表；有详情时左侧候选 + 右侧竖排详情面板
/// （右侧固定宽，二者以竖线分隔）。
fn doc_content(
    scroll_content: Stateful<gpui::Div>,
    doc_state: Option<DocumentationState>,
    selected_item: Option<&CompletionItem>,
    theme: EditorTheme,
    font: &gpui::Font,
    font_size: gpui::Pixels,
    scroll_h: gpui::Pixels,
) -> gpui::Div {
    let row = div().flex().flex_row().w_full().h_full();
    match doc_state {
        Some(doc) => row
            .child(scroll_content)
            .child(div().w(px(COMPLETION_DOC_DIVIDER)).h_full().bg(theme.line_number))
            .child(completion_doc_panel(
                doc,
                selected_item,
                theme,
                font,
                font_size,
                scroll_h,
            )),
        None => row.child(scroll_content),
    }
}

/// F005：右侧 metadata 详情面板。loading/success/error 三态渲染，超出滚动。
fn completion_doc_panel(
    doc: DocumentationState,
    selected_item: Option<&CompletionItem>,
    theme: EditorTheme,
    font: &gpui::Font,
    font_size: gpui::Pixels,
    height: gpui::Pixels,
) -> impl IntoElement {
    let panel = div()
        .id(ElementId::from("completion-doc-panel"))
        .w(px(COMPLETION_DOC_WIDTH))
        .h(height)
        .overflow_hidden()
        // 与左侧候选列表隔开一点，右侧面板整体加点左边距，视觉不贴分隔线。
        .pl_2()
        .flex()
        .flex_col();

    match doc {
        DocumentationState::Ready(text)
            if selected_item.is_some_and(|item| {
                matches!(item.kind, fluxdb_editor_core::CompletionKind::Table)
            }) => panel.child(completion_doc_schema_table(
            selected_item.expect("table completion item").label.as_str(),
            &text,
            theme,
            font,
            font_size,
        )),
        DocumentationState::Loading => panel.child(completion_doc_message(
            "加载中…",
            theme.completion_detail,
            font,
            font_size,
        )),
        DocumentationState::Ready(text) => {
            panel.child(completion_doc_message(
                text.as_str(),
                theme.completion_text,
                font,
                font_size,
            ))
        }
        DocumentationState::Error(reason) => panel.child(completion_doc_message(
            reason.as_str(),
            theme.completion_detail,
            font,
            font_size,
        )),
    }
}

fn completion_doc_schema_table(
    table_name: &str,
    text: &str,
    theme: EditorTheme,
    font: &gpui::Font,
    font_size: gpui::Pixels,
) -> impl IntoElement {
    let rows = completion_doc_rows(text);
    let mut body = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .scrollbar_width(px(9.));
    for (name, type_name, comment) in rows {
        let row = div()
            .h(px(COMPLETION_ROW_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .child(completion_doc_cell(
                name,
                px(128.),
                false,
                theme.completion_text,
                font,
                font_size,
            ))
            .child(completion_doc_cell(
                type_name,
                px(112.),
                true,
                theme.completion_detail,
                font,
                font_size,
            ))
            .child(completion_doc_cell(
                comment,
                px(0.),
                false,
                theme.completion_text,
                font,
                font_size,
            ));
        body = body.child(row);
    }

    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(38.))
                .px_3()
                .flex()
                .items_center()
                .border_b_1()
                .border_color(theme.line_number)
                .text_size((font_size * 0.9).max(px(10.)))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.completion_text)
                .child(format!("表名：{table_name}")),
        )
        .child(completion_doc_table_header(theme, font_size))
        .child(body)
}

fn completion_doc_table_header(theme: EditorTheme, font_size: gpui::Pixels) -> Div {
    div()
        .h(px(28.))
        .w_full()
        .flex()
        .items_center()
        .bg(theme.completion_selected_bg)
        .border_b_1()
        .border_color(theme.line_number)
        .child(completion_doc_header_cell("列名", px(128.), font_size, theme))
        .child(completion_doc_header_cell("数据类型", px(112.), font_size, theme))
        .child(completion_doc_header_cell("注释", px(0.), font_size, theme))
}

fn completion_doc_header_cell(
    text: &'static str,
    width: gpui::Pixels,
    font_size: gpui::Pixels,
    theme: EditorTheme,
) -> Div {
    let cell = div()
        .min_w(px(0.))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size((font_size * 0.85).max(px(9.)))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.completion_detail)
        .child(text);
    if width == px(0.) {
        cell.flex_1()
    } else {
        cell.flex_none().w(width)
    }
}

fn completion_doc_cell(
    text: String,
    width: gpui::Pixels,
    mono: bool,
    color: gpui::Rgba,
    font: &gpui::Font,
    font_size: gpui::Pixels,
) -> Div {
    let cell = div()
        .min_w(px(0.))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size((font_size * 0.85).max(px(9.)))
        .text_color(color)
        .child(StyledText::new(text.clone()).with_runs(vec![text_run(
            text.len(),
            color.into(),
            font,
        )]));
    let cell = if mono { cell.font_family("Menlo") } else { cell };
    if width == px(0.) {
        cell.flex_1()
    } else {
        cell.flex_none().w(width)
    }
}

fn completion_doc_message(
    text: &str,
    color: gpui::Rgba,
    font: &gpui::Font,
    font_size: gpui::Pixels,
) -> impl IntoElement {
    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .p_2()
        .text_size((font_size * 0.85).max(px(9.)))
        .text_color(color)
        .child(StyledText::new(text.to_string()).with_runs(vec![text_run(
            text.len(),
            color.into(),
            font,
        )]))
}

fn completion_doc_rows(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, "  ");
            let name = parts.next()?.trim();
            if name.is_empty() {
                return None;
            }
            Some((
                name.to_string(),
                parts.next().unwrap_or_default().trim().to_string(),
                parts.next().unwrap_or_default().trim().to_string(),
            ))
        })
        .collect()
}

#[cfg(test)]
mod completion_doc_tests {
    use super::completion_doc_rows;

    #[test]
    fn schema_documentation_is_split_into_stable_columns() {
        assert_eq!(
            completion_doc_rows("id  varchar(64)  主键\ncreated_at  timestamp(6)"),
            vec![
                ("id".to_string(), "varchar(64)".to_string(), "主键".to_string()),
                (
                    "created_at".to_string(),
                    "timestamp(6)".to_string(),
                    String::new(),
                ),
            ]
        );
    }
}

/// 单行候选：图标 + 高亮 label（+ 歧义时 detail）。
///
/// 行自带稳定 ElementId（滚动容器即时子节点，供 scroll_to_item 按下标命中），并
/// 处理 hover 高亮与左键接受；点击时会 stop_propagation 阻断落到编辑器正文。
fn completion_row(
    index: usize,
    item: CompletionItem,
    editor: gpui::Entity<Editor>,
    selected: usize,
    ambiguous: &BTreeSet<String>,
    colors: EditorTheme,
    font: &gpui::Font,
    font_size: gpui::Pixels,
    query: &str,
) -> impl IntoElement {
    let is_selected = index == selected;
    let show_detail = ambiguous.contains(&item.label.to_ascii_lowercase());
    let (label_text, label_runs) = highlight_label_runs(&item.label, query, &colors, font);
    let hover_editor = editor.clone();
    let accept_editor = editor.clone();

    let mut row = div()
        .id(ElementId::from(index))
        .h(px(COMPLETION_ROW_HEIGHT))
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .cursor_pointer()
        .text_size(font_size)
        .on_mouse_move(move |_, _, cx| {
            hover_editor.update(cx, |editor, cx| {
                if editor.completion_selected != index {
                    editor.completion_selected = index;
                    // F005：悬停切换选中项同样异步刷新右侧详情并丢弃旧结果。
                    editor.request_completion_documentation(cx);
                    cx.notify();
                }
            });
        })
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            let item = accept_editor.read(cx).completion_items.get(index).cloned();
            accept_editor.update(cx, |editor, cx| {
                editor.completion_selected = index;
                if let Some(item) = item {
                    editor.accept_completion(item, cx);
                }
            });
            cx.stop_propagation();
        })
        .child(
            svg()
                .size(px(16.))
                .path(app_icon_path(completion_kind_icon(item.kind)))
                .text_color(colors.completion_text),
        )
        .child(
            // label 占满剩余空间并裁剪，避免长标识符撑破浮层宽度。
            div().flex_1().overflow_hidden().child(
                StyledText::new(label_text).with_runs(label_runs),
            ),
        );
    if is_selected {
        row = row.bg(colors.completion_selected_bg);
    }
    if show_detail && !item.detail.is_empty() {
        row = row.child(
            div()
                .child(
                    StyledText::new(item.detail.clone()).with_runs(vec![TextRun {
                        len: item.detail.len(),
                        font: font.clone(),
                        color: colors.completion_detail.into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }]),
                )
                .text_size((font_size * 0.8).max(px(9.))),
        );
    }
    row
}
