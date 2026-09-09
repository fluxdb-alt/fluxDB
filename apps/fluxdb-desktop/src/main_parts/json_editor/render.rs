// json_editor/render.rs —— JSON 编辑器主体渲染：查看态结构化行（折叠/层级线/错误高亮）
// 与编辑态（复用 `InputState::code_editor` 全文编辑 + 错误横幅）。
//
// 该面板只负责「选择 JSON 分支并渲染」，不直接连接 Redis；保存走 `request_redis_key_value_apply`。

/// 查看态一条可见行的元信息。
struct JsonViewRow {
    /// 该行在 display 文本中的行号（0 起）。
    line: usize,
    /// 缩进层数（由行首空格数 / indent_size 得出）。
    depth: usize,
    /// 该行 clamp 后的纯文本（用于渲染/复制，不含行首缩进）。
    text: String,
    /// 当前该节点是否处于折叠态。
    folded: bool,
    /// 该行是否处于错误诊断行。
    is_error: bool,
    /// 该行起始节点对应的折叠路径；`None` 表示该行不是可折叠节点的起始行。
    /// 折叠点击直接携带路径，避免用「行号反查节点」在折叠后行号重建时产生身份错位。
    fold_path: Option<JsonFoldPath>,
}

/// 将编辑器 display 文本拆成结构化行列表，供查看态渲染。
fn json_editor_rows(state: &JsonEditorState) -> Vec<JsonViewRow> {
    let indent_size = state.config.indent_size.max(1);
    let display = if state.display.is_empty() {
        state.source.clone()
    } else {
        state.display.clone()
    };
    let mut lines = display.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        lines = vec![""];
    }

    // 折叠节点按起始行索引：标记折叠态并给起始行携带可点击的 fold_path。
    let mut node_by_line = std::collections::HashMap::new();
    for node in &state.nodes {
        node_by_line.insert(node.start_line, node);
    }

    // 诊断错误行：diagnostic.line 是「未折叠 source」中的行号，仅在无折叠时线性对齐到 display。
    // 存在折叠时行号可能错位，因此只在没有任何折叠节点时启用行级错误高亮；并受 `diagnostics` 配置开关控制。
    let any_fold = state.nodes.iter().any(|n| n.folded);
    let mut error_line = None;
    if state.config.diagnostics && !any_fold {
        if let Some(dia) = &state.diagnostic {
            error_line = Some(dia.line);
        }
    }

    lines
        .into_iter()
        .enumerate()
        .map(|(idx, raw)| {
            let leading = raw.len() - raw.trim_start().len();
            let depth = leading / indent_size;
            let text = raw.trim_start().to_string();
            let node = node_by_line.get(&idx).copied();
            let folded = node.map(|n| n.folded).unwrap_or(false);
            JsonViewRow {
                line: idx,
                depth,
                text,
                folded,
                is_error: error_line == Some(idx),
                fold_path: node.map(|n| n.path.clone()),
            }
        })
        .collect()
}

/// JSON token 种类，用于语法高亮。与 Navicat 对齐：完整字符串/数字/布尔/null 各成一个 token，
/// 防止按单字符切分导致 `"付苏维"` 只有引号上色、邮箱里的 `3` 被误当数字染绿。
#[derive(Clone, Copy, PartialEq, Eq)]
enum JsonTokKind {
    /// key token：`"xxx":`（含冒号，不含其后空白）。
    Key,
    /// 完整字符串 value token（含首尾双引号）。
    String,
    /// 完整 number token（数字、`.`、`e/E`、正负号）。
    Number,
    /// 完整 `true` / `false` / `null` 字面量。
    BoolNull,
    /// 标点：`{` `}` `[` `]` `:` `,` 单字符。
    Punct,
    /// 空白段（保留原样，不吞噬）。渲染时通常不需要单独上色。
    Whitespace,
}

/// 一个 JSON token：文本 + 高亮种类。
struct JsonTok {
    text: String,
    kind: JsonTokKind,
}

/// 将行文本按 JSON token 粗切分，供高亮渲染。
///
/// - key：`"xxx":`（保留冒号，但**不吞掉**冒号后的空格，展示为 `"name": "付苏维"`）。
/// - string / number / boolean / null：完整 token。
/// - 空白：保留为单独 token，避免 pretty 的空格被吃掉。
fn json_tokenize_line(text: &str) -> Vec<JsonTok> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let c = rest.chars().next().unwrap();
        if c.is_whitespace() {
            // 空白段：长度取到下一个非空白字符。
            let end = rest
                .char_indices()
                .find(|(_, ch)| !ch.is_whitespace())
                .map(|(i, _)| i)
                .unwrap_or(rest.len());
            out.push(JsonTok { text: rest[..end].to_string(), kind: JsonTokKind::Whitespace });
            rest = &rest[end..];
        } else if c == '"' {
            // 字符串：读到闭合引号（处理 `\` 转义）。若紧跟 `:` 则为 key。
            let len = json_string_len(rest);
            if rest[len..].starts_with(':') {
                out.push(JsonTok { text: rest[..len + 1].to_string(), kind: JsonTokKind::Key });
                rest = &rest[len + 1..];
            } else {
                out.push(JsonTok { text: rest[..len].to_string(), kind: JsonTokKind::String });
                rest = &rest[len..];
            }
        } else if c.is_ascii_digit() || c == '-' {
            // number token：数字、`.`、`e/E`、`+/-`。
            let len = json_number_len(rest);
            out.push(JsonTok { text: rest[..len].to_string(), kind: JsonTokKind::Number });
            rest = &rest[len..];
        } else if c.is_ascii_alphabetic() {
            // boolean / null 字面量；其它字母单词视为标点/中性色。
            let len = json_word_len(rest);
            let word = &rest[..len];
            let kind = if matches!(word, "true" | "false" | "null") {
                JsonTokKind::BoolNull
            } else {
                JsonTokKind::Punct
            };
            out.push(JsonTok { text: word.to_string(), kind });
            rest = &rest[len..];
        } else {
            // 单个字符标点（`{` `}` `[` `]` `:` `,` 等）。
            let len = c.len_utf8();
            out.push(JsonTok { text: rest[..len].to_string(), kind: JsonTokKind::Punct });
            rest = &rest[len..];
        }
    }
    if out.is_empty() {
        out.push(JsonTok { text: text.to_string(), kind: JsonTokKind::Punct });
    }
    out
}

/// 读取一个 JSON 字符串 token 的字节长度（含首尾双引号）；正确处理 `\` 转义。
fn json_string_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 1; // 跳过开引号
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            // 跳过后一个转义字符（按其 utf8 宽度），避免把转义后的引号当成闭合。
            i += 1;
            if i < bytes.len() {
                let ch = std::str::from_utf8(&bytes[i..])
                    .ok()
                    .and_then(|r| r.chars().next());
                i += ch.map(|c| c.len_utf8()).unwrap_or(1);
            }
            continue;
        }
        if b == b'"' {
            return i + 1;
        }
        i += 1;
    }
    s.len() // 未闭合的字符串（非法 JSON），按整段处理
}

/// 读取 number token 的字节长度（ASCII）。
fn json_number_len(s: &str) -> usize {
    let mut n = 0;
    for b in s.as_bytes() {
        if b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-') {
            n += 1;
        } else {
            break;
        }
    }
    n
}

/// 读取字母单词（boolean/null 字面量）的字节长度（ASCII）。
fn json_word_len(s: &str) -> usize {
    s.bytes().take_while(|b| b.is_ascii_alphabetic()).count()
}
fn redis_json_value_panel(
    tab_id: TabId,
    detail: &RedisKeyDetail,
    applying: bool,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let key = detail.key.clone();
    let state = this.redis_string_values.get(&(tab_id, key.clone())).cloned();
    let loaded_all = state.as_ref().map(|state| state.loaded_all).unwrap_or(false);
    let len = state.as_ref().map(|state| state.len);
    let loading = this.redis_json_editor_busy
        || this
            .redis_string_loading
            .as_ref()
            .is_some_and(|loading| loading.0 == tab_id && loading.1 == key);
    let editing = this.redis_json_editor.editing && this.redis_json_editor_is_active(tab_id, &key);
    let active_editable = this.redis_json_editor.config.editable
        && loaded_all
        && !applying
        && !editing
        && !redis_value_is_binary(&detail.value);

    redis_detail_panel(colors)
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(
            div()
                .h(px(28.))
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .child(redis_detail_panel_title("值", colors))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(redis_string_value_hint(len, loaded_all, loading, colors))
                        .child(redis_json_toolbar_format_button(
                            editing,
                            applying,
                            colors,
                            cx,
                        ))
                        .child(redis_string_copy_button(
                            tab_id,
                            key.clone(),
                            this.redis_json_editor.source.clone(),
                            loaded_all && !redis_value_is_binary(&detail.value),
                            redis_string_copy_disabled_tooltip(
                                redis_value_is_binary(&detail.value),
                                loaded_all,
                            ),
                            colors,
                            cx,
                        ))
                        .child(redis_string_download_button(
                            tab_id,
                            detail.clone(),
                            !applying,
                            colors,
                            cx,
                        ))
                        .child(redis_json_edit_button(active_editable, colors, cx)),
                ),
        )
        .child(redis_json_value_body(
            tab_id,
            detail.clone(),
            editing,
            loaded_all,
            loading,
            applying,
            this,
            colors,
            cx,
        ))
}

/// 顶部「格式化」按钮：编辑态可用；合法 JSON pretty 回填，非法不改写并提示。
fn redis_json_toolbar_format_button(
    editing: bool,
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let enabled = editing && !applying;
    div()
        .h(px(24.))
        .px_1()
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .text_color(if enabled { colors.text } else { colors.muted })
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |style| style.bg(colors.hover))
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                if editing && !applying {
                    this.redis_json_format(window, cx);
                }
                cx.stop_propagation();
            }),
        )
        .child(app_icon(
            AppIcon::AlignLeft,
            13.,
            if enabled { colors.muted } else { colors.border },
        ))
        .child("格式化")
}

/// 编辑按钮：完整加载 + 非二进制 + 非保存/编辑中才可进入编辑态。
fn redis_json_edit_button(
    enabled: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(24.))
        .px_1()
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .text_color(if enabled { colors.text } else { colors.muted })
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |style| style.bg(colors.hover))
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                if enabled {
                    this.begin_redis_json_edit(window, cx);
                }
                cx.stop_propagation();
            }),
        )
        .child(app_icon(
            AppIcon::Edit,
            13.,
            if enabled { colors.muted } else { colors.border },
        ))
        .child("编辑")
}

/// JSON 编辑器主体：加载中 → 编辑态 → 未完整加载（预览 + 加载全部）→ 完整加载查看态（结构化行）。
fn redis_json_value_body(
    tab_id: TabId,
    detail: RedisKeyDetail,
    editing: bool,
    loaded_all: bool,
    loading: bool,
    applying: bool,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let placeholder = this.redis_json_editor.config.placeholder.clone();
    let body = div().flex_1().min_h(px(0.)).overflow_hidden();
    if loading {
        return body.child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(loading_spinner_with_color(22., colors.muted)),
        );
    }

    if editing {
        return body.child(redis_json_edit_body(
            tab_id,
            detail,
            applying,
            this,
            colors,
            cx,
        ));
    }

    if !loaded_all {
        // 未完整加载：展示 preview + 省略号 + 「加载全部」，避免把截断预览当完整值编辑/保存。
        let preview = this.redis_json_editor.source.clone();
        return body.child(
            div()
                .h_full()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .id(("redis-json-value-preview-scroll", tab_id.0))
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_x_scroll()
                        .overflow_y_scrollbar()
                        .font_family("Menlo")
                        .text_size(px(13.))
                        .line_height(px(19.))
                        .text_color(colors.text)
                        .child(
                            div().px_3().py_2().child(if preview.is_empty() {
                                div()
                                    .text_color(colors.muted)
                                    .child(placeholder)
                                    .into_any_element()
                            } else {
                                format!("{preview}\n…").into_any_element()
                            }),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .px_3()
                        .py_2()
                        .flex()
                        .items_center()
                        .justify_end()
                        .child(
                            Button::new(("redis-json-value-load-all", tab_id.0))
                                .label("加载全部")
                                .small()
                                .outline()
                                .w(px(88.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.request_redis_string_value_load(
                                        tab_id,
                                        detail.key.clone(),
                                        true,
                                        cx,
                                    );
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        );
    }

    // 完整加载查看态：结构化行（折叠 / 层级线 / 错误高亮）。
    body.child(redis_json_view_rows(tab_id, this, colors, cx))
}

/// 编辑态主体：InputState::code_editor 全文编辑（内建红波浪下划线 + 悬停气泡）+ 取消/保存。
/// 错误提示仅依赖编辑器内下划线 + 悬停气泡，底部不渲染任何提示行。
fn redis_json_edit_body(
    tab_id: TabId,
    detail: RedisKeyDetail,
    applying: bool,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let input = this.redis_json_key_value_input.clone();
    let theme = JsonEditorTheme::from_colors(colors);
    let diagnostic = this.redis_json_editor.diagnostic.clone();
    // `min_rows`：编辑器最小可见行数（px），编辑区低于此高度也保持可读。
    let min_rows_px = (this.redis_json_editor.config.min_rows.max(4) as f32) * JSON_ROW_HEIGHT;
    // 两个 `move` 闭包各自持有 detail 副本（取消 / 保存）。
    let detail_cancel = detail.clone();
    let detail_save = detail.clone();

    div()
        .h_full()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .px_3()
        .py_2()
        .gap_2()
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .w_full()
                .child(
                    div()
                        .w_full()
                        .h_full()
                        .min_h(px(min_rows_px))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(if diagnostic.is_some() {
                            theme.error_border
                        } else {
                            colors.border
                        })
                        .bg(colors.input_bg)
                        .overflow_hidden()
                        .child(
                            Input::new(&input)
                                .appearance(false)
                                .focus_bordered(false)
                                .disabled(applying)
                                .w_full()
                                .h_full()
                                .px_2()
                                .py_2()
                                .font_family("Menlo")
                                .text_size(px(13.))
                                .line_height(px(19.)),
                        ),
                ),
        )
        // 错误提示完全位于编辑器内：错误 token 处的红波浪下划线 + 悬停气泡，
        // 底部不再放置任何提示行/横幅。
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .child(
                    Button::new(("redis-json-value-cancel-edit", tab_id.0))
                        .label("取消编辑")
                        .small()
                        .outline()
                        .w(px(82.))
                        .disabled(applying)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if !applying {
                                this.cancel_redis_json_edit(tab_id, &detail_cancel, window, cx);
                            }
                            cx.stop_propagation();
                        })),
                )
                // JSON 非法时保存按钮降权为弱样式并禁用，避免强主按钮误导；
                // 仍保留 apply 侧二次校验兜底。
                .child(redis_json_save_button(
                    tab_id,
                    diagnostic.clone(),
                    applying,
                    detail_save,
                    cx,
                )),
        )
}

/// 保存按钮：JSON 非法时降权为弱样式并禁用，避免强主按钮误导；仍保留 apply 侧二次校验兜底。
fn redis_json_save_button(
    tab_id: TabId,
    diagnostic: Option<JsonEditorDiagnostic>,
    applying: bool,
    detail: RedisKeyDetail,
    cx: &mut Context<NavicatMain>,
) -> Button {
    let invalid = diagnostic.is_some();
    let mut button = Button::new(("redis-json-value-save", tab_id.0))
        .label("保存")
        .small()
        .w(px(72.))
        .disabled(applying || invalid)
        .on_click(cx.listener(move |this, _, _, cx| {
            if !applying {
                this.request_redis_key_value_apply(tab_id, detail.clone(), cx);
            }
            cx.stop_propagation();
        }));
    if invalid {
        button = button.outline();
    } else {
        button = button.primary();
    }
    button
}

/// 完整加载查看态：按 pretty 行渲染，带一致性 gutter（折叠按钮 + 层级线）与错误行高亮。
fn redis_json_view_rows(
    tab_id: TabId,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &Context<NavicatMain>,
) -> gpui::Stateful<Div> {
    let theme = JsonEditorTheme::from_colors(colors);
    let cfg = &this.redis_json_editor.config;
    let indent_size = cfg.indent_size.max(1);
    let rows = json_editor_rows(&this.redis_json_editor);
    let show_gutter = cfg.show_gutter && cfg.folding;
    let enable_fold = cfg.folding;
    let highlight = cfg.syntax_highlight;
    let show_numbers = cfg.line_numbers;

    // 紧凑 gutter 列：融合「行号 + 折叠图标」为同一容器，顺序 [行号][折叠图标][正文]。
    // gutter 背景透明（与编辑器背景一致），不再使用独立大灰色折叠栏。
    // 折叠状态由 `JsonViewRow.fold_path` 直接携带（行号在折叠后会重建，不能反查节点）。
    let gutter_col = div()
        .flex_none()
        .flex()
        .flex_col()
        .children(rows.iter().map(|row| {
            redis_json_gutter_row(
                row,
                enable_fold.then(|| row.fold_path.clone()).flatten(),
                show_numbers,
                theme,
                colors,
                cx,
            )
        }));

    // 正文列：随 gutter 之后开始渲染，行高一致以纵向对齐。
    let text_col = div().flex_1().min_w(px(0.)).children(
        rows.iter()
            .map(|row| redis_json_text_row(row, theme, indent_size, highlight)),
    );
    let view = div()
        .id(("redis-json-value-view", tab_id.0))
        .size_full()
        .flex()
        .bg(colors.input_bg);
    if show_gutter {
        // 显示折叠图标时同时保留行号（若开启）；行号 + 折叠图标在同一个 gutter 容器内。
        view.child(gutter_col).child(text_col)
    } else if show_numbers {
        // 不显示折叠但显示行号：仍在一个紧凑的行号列内。
        let num_only = div().flex_none().flex().flex_col().children(
            rows.iter().map(|row| {
                div()
                    .h(px(JSON_ROW_HEIGHT))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .w(px(JSON_LINE_NUM_WIDTH))
                    .pr_1()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child((row.line + 1).to_string())
            }),
        );
        view.child(num_only).child(text_col)
    } else {
        view.child(text_col)
    }
}

/// 查看态单行高度（与 gutter / 正文两列一致，保证垂直对齐）。
const JSON_ROW_HEIGHT: f32 = 19.0;

/// 行号列宽度（约 36px，右对齐，符合 VS Code 紧凑布局）。
const JSON_LINE_NUM_WIDTH: f32 = 36.0;

/// 折叠图标列宽度（约 17px，紧贴行号右侧，间距 4-6px）。
const JSON_FOLD_COL_WIDTH: f32 = 17.0;

/// gutter 行：融合「行号（可选）+ 折叠图标」为一行，顺序 [行号][折叠图标][正文]。
///
/// - 行号列右对齐灰色，宽度 `JSON_LINE_NUM_WIDTH`。
/// - 折叠图标列紧贴行号右侧，间距 4-6px；无折叠节点的行保留列宽但不显示图标，保证正文对齐。
/// - 整行作为折叠命中区，悬停高亮；gutter 背景透明，与编辑器背景一致。
/// - 折叠点击直接携带 `fold_path`（`Some(空)` 表示根节点也可折叠），不再用行号反查节点。
#[allow(clippy::too_many_arguments)]
fn redis_json_gutter_row(
    row: &JsonViewRow,
    fold_path: Option<JsonFoldPath>,
    show_numbers: bool,
    theme: JsonEditorTheme,
    colors: UiColors,
    cx: &Context<NavicatMain>,
) -> gpui::Stateful<Div> {
    // 是否为可折叠节点起始行（`Some` 即代表可折叠；根节点的路径为空 `Vec` 同样可折叠）。
    let is_fold_start = fold_path.is_some();
    div()
        .h(px(JSON_ROW_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .id(("redis-json-gutter-row", row.line))
        .when(is_fold_start, |this| {
            // 整行作为折叠命中区（gutter 点击即折叠/展开）；`fold_path` 已随行携带，克隆供移动闭包使用。
            let path = fold_path.clone().expect("fold start must carry path");
            this.hover(move |s| s.bg(theme.fold_hover))
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.redis_json_toggle_fold(path.clone(), window, cx);
                        cx.stop_propagation();
                    }),
                )
        })
        // 行号列（右对齐）。
        .when(show_numbers, |this| {
            this.child(
                div()
                    .w(px(JSON_LINE_NUM_WIDTH))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .pr_1()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child((row.line + 1).to_string()),
            )
        })
        // 折叠图标列：固定宽度，紧贴行号右侧；无折叠节点时留空保持对齐。
        .child(
            div()
                .w(px(JSON_FOLD_COL_WIDTH))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(if is_fold_start {
                    app_icon(
                        if row.folded { AppIcon::ChevronRight } else { AppIcon::ChevronDown },
                        12.,
                        colors.text,
                    )
                    .into_any_element()
                } else {
                    div().into_any_element()
                }),
        )
}

/// 正文行：层级竖线 + 语法高亮 token 渲染。`highlight` 为 false 时统一正文色（关闭语法高亮）。
///
/// `config.syntax_highlight` 仅作用于**查看态**的 token 配色（本函数）。**编辑态**复用
/// `InputState::code_editor` 全文编辑，而项目当前对其未配置任何语言（`Input::new` 普通文本渲染，
/// 未调用 `.code_editor(language)`），因此编辑态无 token 高亮可开关——`syntax_highlight=false`
/// 无法在组件局部关闭编辑态高亮，属 gpui-component 层限制，非本项目缺口，不做大重构。
///
/// 用单个 `StyledText` 按 token 字节范围上色：空白段不设高亮、继承默认文字色，
/// 从而完整保留 pretty 输出中的空格（如 `"name": "付苏维"`），避免多余 div 吃空格。
fn redis_json_text_row(
    row: &JsonViewRow,
    theme: JsonEditorTheme,
    indent_size: usize,
    highlight: bool,
) -> Div {
    let toks = json_tokenize_line(&row.text);
    let indent_px = (indent_size as f32) * 8.0; // Menlo 13px 等宽近似
    let row_bg = if row.is_error {
        theme.error_bg
    } else {
        gpui::Rgba { r: 0., g: 0., b: 0., a: 0. }
    };

    // 构建 token 高亮区间：仅给非空白 token 上色，区间按字节、升序、不重叠。
    let mut text = String::new();
    let mut highlights: Vec<(std::ops::Range<usize>, gpui::HighlightStyle)> = Vec::new();
    for tok in &toks {
        if tok.kind != JsonTokKind::Whitespace {
            let color = if highlight {
                color_for_json_token(tok, theme)
            } else {
                theme.key
            };
            let start = text.len();
            text.push_str(&tok.text);
            highlights.push((
                start..text.len(),
                gpui::HighlightStyle { color: Some(color.into()), ..Default::default() },
            ));
        } else {
            // 空白段：不设高亮，继承默认文字色，并保持原样写入文本。
            text.push_str(&tok.text);
        }
    }

    div()
        .h(px(JSON_ROW_HEIGHT))
        .flex_none()
        .relative()
        .flex()
        .items_center()
        .bg(row_bg)
        .pl(px(indent_px * (row.depth as f32)))
        .pr_1()
        .when(row.is_error, |this| {
            this.border_l_2().border_color(theme.error_border)
        })
        .child(
            div()
                .font_family("Menlo")
                .text_size(px(13.))
                .line_height(px(JSON_ROW_HEIGHT))
                .child(
                    gpui::StyledText::new(text.clone())
                        .with_highlights(highlights),
                ),
        )
        // 层级竖线：在当前行左侧按缩进层数画竖条，跨行拼接成连续导线。
        .children(
            (0..row.depth.min(64)).filter(|d| *d > 0).map(move |d| {
                div()
                    .absolute()
                    .left(px(indent_px * (d as f32) - 1.))
                    .top_0()
                    .bottom_0()
                    .w(px(1.))
                    .bg(theme.guide)
            }),
        )
}
/// 根据 token 种类推断语法颜色。优先 key/字符串/数字/布尔/null/标点。
fn color_for_json_token(tok: &JsonTok, theme: JsonEditorTheme) -> gpui::Rgba {
    match tok.kind {
        JsonTokKind::Key => theme.key,
        JsonTokKind::String => theme.string,
        JsonTokKind::Number => theme.number,
        JsonTokKind::BoolNull => theme.boolean,
        // 空白在渲染时被跳过、不进入这里；标点用弱化中性色。
        JsonTokKind::Punct | JsonTokKind::Whitespace => theme.punctuation,
    }
}
