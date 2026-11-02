// json_editor/render.rs —— JSON 编辑器主体渲染：查看态结构化行（折叠/层级线/错误高亮）
// 与编辑态（复用 `JsonComponent` 的多行编辑框全文编辑）。
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
    let display = if state.display.is_empty() {
        state.source.clone()
    } else {
        state.display.clone()
    };
    // 诊断错误行：diagnostic.line 是「未折叠 source」中的行号，仅在无折叠时线性对齐到 display。
    // 存在折叠时行号可能错位，因此只在没有任何折叠节点时启用行级错误高亮；并受 `diagnostics` 配置开关控制。
    let error_line = (state.config.diagnostics && !state.nodes.iter().any(|n| n.folded))
        .then(|| state.diagnostic.as_ref().map(|dia| dia.line))
        .flatten();
    json_rows_from_display(&display, &state.nodes, state.config.indent_size, error_line)
}

/// 由「展示文本 + 折叠节点」拆出可见行。
///
/// 与 `JsonEditorState` 解耦：MongoDB 文档卡片只有 pretty 文本和节点列表，
/// 没有编辑器状态，也走同一条渲染路径。
fn json_rows_from_display(
    display: &str,
    nodes: &[JsonFoldNode],
    indent_size: usize,
    error_line: Option<usize>,
) -> Vec<JsonViewRow> {
    let indent_size = indent_size.max(1);
    let mut lines = display.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        lines = vec![""];
    }

    // 折叠节点按起始行索引：标记折叠态并给起始行携带可点击的 fold_path。
    let mut node_by_line = std::collections::HashMap::new();
    for node in nodes {
        node_by_line.insert(node.start_line, node);
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
            out.push(JsonTok {
                text: rest[..end].to_string(),
                kind: JsonTokKind::Whitespace,
            });
            rest = &rest[end..];
        } else if c == '"' {
            // 字符串：读到闭合引号（处理 `\` 转义）。若紧跟 `:` 则为 key。
            let len = json_string_len(rest);
            if rest[len..].starts_with(':') {
                out.push(JsonTok {
                    text: rest[..len + 1].to_string(),
                    kind: JsonTokKind::Key,
                });
                rest = &rest[len + 1..];
            } else {
                out.push(JsonTok {
                    text: rest[..len].to_string(),
                    kind: JsonTokKind::String,
                });
                rest = &rest[len..];
            }
        } else if c.is_ascii_digit() || c == '-' {
            // number token：数字、`.`、`e/E`、`+/-`。
            let len = json_number_len(rest);
            out.push(JsonTok {
                text: rest[..len].to_string(),
                kind: JsonTokKind::Number,
            });
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
            out.push(JsonTok {
                text: word.to_string(),
                kind,
            });
            rest = &rest[len..];
        } else {
            // 单个字符标点（`{` `}` `[` `]` `:` `,` 等）。
            let len = c.len_utf8();
            out.push(JsonTok {
                text: rest[..len].to_string(),
                kind: JsonTokKind::Punct,
            });
            rest = &rest[len..];
        }
    }
    if out.is_empty() {
        out.push(JsonTok {
            text: text.to_string(),
            kind: JsonTokKind::Punct,
        });
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
    let state = this
        .redis_string_values
        .get(&(tab_id, key.clone()))
        .cloned();
    let loaded_all = state
        .as_ref()
        .map(|state| state.loaded_all)
        .unwrap_or(false);
    let len = state.as_ref().map(|state| state.len);
    let loading = this.redis_json_editor_busy
        || this
            .redis_string_loading
            .as_ref()
            .is_some_and(|loading| loading.0 == tab_id && loading.1 == key);
    let editing =
        this.redis_json_editor.state().editing && this.redis_json_editor_is_active(tab_id, &key);
    let active_editable = this.redis_json_editor.state().config.editable
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
                .child(redis_detail_panel_title(tr("app.json.value"), colors))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(redis_string_value_hint(len, loaded_all, loading, colors))
                        .child(redis_json_toolbar_format_button(
                            editing, applying, colors, cx,
                        ))
                        .child(redis_string_copy_button(
                            tab_id,
                            key.clone(),
                            this.redis_json_editor.state().source.clone(),
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
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
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
        .child(tr("app.json.format"))
}

/// 编辑按钮：完整加载 + 非二进制 + 非保存/编辑中才可进入编辑态。
fn redis_json_edit_button(enabled: bool, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
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
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
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
        .child(tr("app.json.edit"))
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
    let placeholder = this.redis_json_editor.state().config.placeholder.clone();
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
            tab_id, detail, applying, this, colors, cx,
        ));
    }

    if !loaded_all {
        // 未完整加载：展示 preview + 省略号 + 「加载全部」，避免把截断预览当完整值编辑/保存。
        let preview = this.redis_json_editor.state().source.clone();
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
                        .font_family(crate::PLATFORM_MONOSPACE_FONT)
                        .text_size(px(13.))
                        .line_height(px(19.))
                        .text_color(colors.text)
                        .child(div().px_3().py_2().child(if preview.is_empty() {
                            div()
                                .text_color(colors.muted)
                                .child(placeholder)
                                .into_any_element()
                        } else {
                            format!("{preview}\n…").into_any_element()
                        })),
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
                                .label(tr("app.redis.load_all"))
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

/// 编辑态主体：复用 `JsonComponent` 的 Edit 模式（多行编辑框；单行 `InputState`
/// 会删除换行，导致 pretty 后的 JSON 被压回一行）+ 取消/保存。
fn redis_json_edit_body(
    tab_id: TabId,
    detail: RedisKeyDetail,
    applying: bool,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let diagnostic = this.redis_json_editor.state().diagnostic.clone();
    // `min_rows`：编辑器最小可见行数（px），编辑区低于此高度也保持可读。
    let min_rows_px =
        (this.redis_json_editor.state().config.min_rows.max(4) as f32) * JSON_ROW_HEIGHT;
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
            div().flex_1().min_h(px(0.)).w_full().child(
                div()
                    .w_full()
                    .h_full()
                    .min_h(px(min_rows_px))
                    .child(this.redis_json_editor.render_edit_body(colors, applying)),
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
                        .label(tr("app.redis.string.cancel_edit"))
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
        .label(tr("app.redis.save"))
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
///
/// 渲染统一走 `json_fold_view`（与 MongoDB 文档卡片同一实现），这里只把编辑器的
/// 配置翻译成视图参数，并注入 Redis 自己的折叠回调。
fn redis_json_view_rows(
    tab_id: TabId,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &Context<NavicatMain>,
) -> gpui::Stateful<Div> {
    let state = this.redis_json_editor.state();
    let cfg = &state.config;
    let displayed = if state.display.is_empty() {
        state.source.clone()
    } else {
        state.display.clone()
    };
    let mut view = JsonFoldView::panel(&displayed, &state.nodes, cfg.syntax_highlight);
    view.indent_size = cfg.indent_size.max(1);
    // 诊断行只在无折叠时有效（折叠后行号会重建），与 `json_editor_rows` 的判定保持一致。
    view.error_line = json_editor_rows(state)
        .iter()
        .find(|row| row.is_error)
        .map(|row| row.line);
    // gutter 即「行号 + 折叠图标」这一列，两者随 `show_gutter` 一起开关（与改造前一致）。
    view.line_num_width = (cfg.show_gutter && cfg.line_numbers).then_some(JSON_LINE_NUM_WIDTH);
    view.folding = cfg.show_gutter && cfg.folding;

    let entity = cx.entity().downgrade();
    json_fold_view(&view, colors, move |path, window, cx| {
        let _ = entity.update(cx, |this: &mut NavicatMain, cx| {
            this.redis_json_toggle_fold(path, window, cx);
        });
    })
    .id(("redis-json-value-view", tab_id.0))
    .size_full()
}

/// 只读 JSON 视图（带行号 / 折叠箭头 / 语法高亮）的可配置样式。
///
/// 与 `NavicatMain` 解耦：调用方给一段 pretty 文本 + 折叠节点 + 样式 + 折叠回调，
/// 就能得到完整的可折叠 JSON 视图（Redis 值详情、MongoDB 文档卡片共用）。
pub(crate) struct JsonFoldView<'a> {
    /// pretty 后的展示文本。
    pub display: &'a str,
    /// 可折叠节点（由 `build_pretty` 按同一份折叠集合产出）。
    pub nodes: &'a [JsonFoldNode],
    pub row_height: f32,
    /// 折叠行的真实内容（按当前展示行号）：复制时输出它，而不是屏上的 `{ … }`。
    pub expanded_copy: Option<&'a std::collections::HashMap<usize, String>>,
    /// 行号列宽度；`None` 表示不显示行号列。
    pub line_num_width: Option<f32>,
    pub line_num_text_size: f32,
    pub text_size: f32,
    /// 是否显示折叠箭头并允许折叠。
    pub folding: bool,
    /// 是否做语法高亮。
    pub highlight: bool,
    pub indent_size: usize,
    /// 需要标红的行号（非法 JSON 的诊断行）；`None` 表示不高亮任何行。
    pub error_line: Option<usize>,
    /// 行序基准：该视图首行在窗口选择文档里的序号。
    ///
    /// 同一窗口有多份 JSON 视图时，各视图的行序必须错开，否则选区端点比较会误判方向。
    pub order_base: u64,
    /// 可选的选择器前缀：给出时每个 gutter 行带 `{前缀}:{行号}`，无头用例才能点到折叠箭头。
    pub selector_prefix: Option<String>,
    /// gutter 行元素 id 前缀；同一页面有多份 JSON 视图时必须互不相同。
    pub id_salt: SharedString,
}

impl<'a> JsonFoldView<'a> {
    /// 与 Redis JSON 面板一致的默认样式（19px 行高、36px 行号列、13px 等宽字）。
    pub(crate) fn panel(
        display: &'a str,
        nodes: &'a [JsonFoldNode],
        highlight: bool,
    ) -> JsonFoldView<'a> {
        JsonFoldView {
            display,
            nodes,
            row_height: JSON_ROW_HEIGHT,
            expanded_copy: None,
            line_num_width: Some(JSON_LINE_NUM_WIDTH),
            line_num_text_size: 11.,
            text_size: 13.,
            folding: true,
            highlight,
            indent_size: 2,
            error_line: None,
            order_base: 0,
            selector_prefix: None,
            id_salt: "json-fold-panel".into(),
        }
    }
}

/// 渲染只读 JSON 视图；`on_toggle` 在点击折叠箭头时收到该节点的折叠路径。
///
/// 行拆分走 `json_rows_from_display`，与编辑器查看态同一套逻辑（折叠占位、层级缩进、
/// 折叠后行号重建都由它保证），因此两个调用点的折叠行为不会漂移。
pub(crate) fn json_fold_view(
    view: &JsonFoldView<'_>,
    colors: UiColors,
    on_toggle: impl Fn(JsonFoldPath, &mut Window, &mut App) + 'static,
) -> Div {
    let theme = JsonEditorTheme::from_colors(colors);
    let rows = json_rows_from_display(view.display, view.nodes, view.indent_size, view.error_line);
    let on_toggle = std::rc::Rc::new(on_toggle);
    let show_numbers = view.line_num_width.is_some();

    let gutter_col = div()
        .flex_none()
        .flex()
        .flex_col()
        .children(rows.iter().map(|row| {
            let toggle = on_toggle.clone();
            let fold_path = row.fold_path.clone();
            let is_fold_start = view.folding && fold_path.is_some();
            // 可折叠行与非折叠行给不同选择器：无头用例据此判断哪一行是折叠箭头，
            // 也能数出当前可见行数（折叠后行号会重建）。
            let gutter_selector = view.selector_prefix.as_ref().map(|prefix| {
                if is_fold_start {
                    format!("{prefix}:{}", row.line)
                } else {
                    format!("{prefix}-plain:{}", row.line)
                }
            });
            div()
                .h(px(view.row_height))
                .flex_none()
                .flex()
                .items_center()
                .id((view.id_salt.clone(), row.line))
                .when_some(gutter_selector, |this, selector| {
                    this.debug_selector(move || selector)
                })
                .when(is_fold_start, |this| {
                    let path = fold_path.clone().expect("fold start must carry path");
                    this.hover(move |s| s.bg(theme.fold_hover))
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            (*toggle)(path.clone(), window, cx);
                            cx.stop_propagation();
                        })
                })
                .when(show_numbers, |this| {
                    this.child(
                        div()
                            .w(px(view.line_num_width.unwrap_or_default()))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_end()
                            .pr_1()
                            .text_size(px(view.line_num_text_size))
                            .text_color(colors.muted)
                            .child((row.line + 1).to_string()),
                    )
                })
                .child(
                    div()
                        .w(px(JSON_FOLD_COL_WIDTH))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(if is_fold_start {
                            app_icon(
                                if row.folded {
                                    AppIcon::ChevronRight
                                } else {
                                    AppIcon::ChevronDown
                                },
                                12.,
                                colors.text,
                            )
                            .into_any_element()
                        } else {
                            div().into_any_element()
                        }),
                )
        }));

    let text_col = div()
        .flex_1()
        .min_w(px(0.))
        .children(rows.iter().map(|row| {
            json_text_row_with_metrics(
                row,
                theme,
                view.indent_size,
                view.highlight,
                view.row_height,
                view.text_size,
                // 每行一个选择参与者：行号即阅读顺序，跨行拖选与复制按它排。
                Some(JsonRowSelection {
                    id: {
                        let id = gpui::ElementId::NamedInteger(
                            SharedString::from(format!("{}:row-text", view.id_salt)),
                            row.line as u64,
                        );
                        // 折叠行的复制内容来自展开态；元素 id 带上折叠态，
                        // 展开/折叠切换后不会继承上一帧的复制回调。
                        if row.folded {
                            (id, "folded").into()
                        } else {
                            id
                        }
                    },
                    // 行号随折叠变化，序号必须带视图基准，才能与其它视图/卡片错开。
                    order: view.order_base + row.line as u64,
                    debug_selector: view
                        .selector_prefix
                        .as_ref()
                        .map(|prefix| format!("{prefix}-text:{}", row.line)),
                    copy_text: if row.folded {
                        view.expanded_copy
                            .and_then(|map| map.get(&row.line))
                            .cloned()
                            .map(SharedString::from)
                    } else {
                        None
                    },
                }),
            )
        }));
    div()
        .flex()
        .bg(colors.input_bg)
        .child(gutter_col)
        .child(text_col)
}

/// 查看态单行高度（与 gutter / 正文两列一致，保证垂直对齐）。
const JSON_ROW_HEIGHT: f32 = 19.0;

/// 行号列宽度（约 36px，右对齐，符合 VS Code 紧凑布局）。
const JSON_LINE_NUM_WIDTH: f32 = 36.0;

/// 折叠图标列宽度（约 17px，紧贴行号右侧，间距 4-6px）。
const JSON_FOLD_COL_WIDTH: f32 = 17.0;

/// 正文行：层级竖线 + 语法高亮 token 渲染。`highlight` 为 false 时统一正文色（关闭语法高亮）。
///
/// `config.syntax_highlight` 仅作用于**查看态**的 token 配色（本函数）。**编辑态**由
/// `JsonComponent` 内部的 `EditorState` 提供 JSON 语法高亮，配色来自主题而非该开关，
/// 因此 `syntax_highlight=false` 只影响查看态。
///
/// 用单个 `StyledText` 按 token 字节范围上色：空白段不设高亮、继承默认文字色，
/// 从而完整保留 pretty 输出中的空格（如 `"name": "付苏维"`），避免多余 div 吃空格。
fn redis_json_text_row(
    row: &JsonViewRow,
    theme: JsonEditorTheme,
    indent_size: usize,
    highlight: bool,
) -> Div {
    json_text_row_with_metrics(row, theme, indent_size, highlight, JSON_ROW_HEIGHT, 13., None)
}

/// 一行文本的选择参数：行内元素 id、阅读顺序、调试选择器与复制内容。
struct JsonRowSelection {
    id: gpui::ElementId,
    order: u64,
    /// 无头用例定位行内文本的调试选择器；只在测试期记录位置。
    debug_selector: Option<String>,
    /// 折叠行的真实内容：复制时用它替换屏上的 `{ … }`。
    copy_text: Option<SharedString>,
}

/// 正文行的可配置版本：行高与字号由调用方决定（卡片视图比 Redis 面板小一号）。
///
/// `selection` 给出时该行接入文本选择（双击选词、拖选跨行、复制）；`None` 保持纯展示。
fn json_text_row_with_metrics(
    row: &JsonViewRow,
    theme: JsonEditorTheme,
    indent_size: usize,
    highlight: bool,
    row_height: f32,
    text_size: f32,
    selection: Option<JsonRowSelection>,
) -> Div {
    let debug_selector = selection
        .as_ref()
        .and_then(|selection| selection.debug_selector.clone());
    let toks = json_tokenize_line(&row.text);
    // 13px 等宽字下一层缩进约 8px；其它字号按比例缩放，卡片视图（11px）也保持层级清晰。
    let indent_px = (indent_size as f32) * text_size * (8.0 / 13.0);
    let row_bg = if row.is_error {
        theme.error_bg
    } else {
        gpui::Rgba {
            r: 0.,
            g: 0.,
            b: 0.,
            a: 0.,
        }
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
                gpui::HighlightStyle {
                    color: Some(color.into()),
                    ..Default::default()
                },
            ));
        } else {
            // 空白段：不设高亮，继承默认文字色，并保持原样写入文本。
            text.push_str(&tok.text);
        }
    }

    div()
        .h(px(row_height))
        .flex_none()
        .relative()
        .flex()
        .items_center()
        .bg(row_bg)
        .pl(px(indent_px * (row.depth as f32)))
        .pr_1()
        .when_some(debug_selector, |this, selector| {
            this.debug_selector(move || selector)
        })
        .when(row.is_error, |this| {
            this.border_l_2().border_color(theme.error_border)
        })
        .child({
            let text_container = div()
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_size(px(text_size))
                .line_height(px(row_height));
            match selection {
                // 可选中版本自己画高亮：`SelectableText` 只吃纯文本，会丢掉 token 配色。
                Some(selection) => {
                    let mut element = SelectableStyledText::new(selection.id, text.clone())
                        .highlights(highlights)
                        .document_order(selection.order);
                    if let Some(copy_text) = selection.copy_text {
                        element = element.copy_text(copy_text);
                    }
                    text_container.child(element)
                }
                None => text_container
                    .child(gpui::StyledText::new(text.clone()).with_highlights(highlights)),
            }
        })
        // 层级竖线：在当前行左侧按缩进层数画竖条，跨行拼接成连续导线。
        .children((0..row.depth.min(64)).filter(|d| *d > 0).map(move |d| {
            div()
                .absolute()
                .left(px(indent_px * (d as f32) - 1.))
                .top_0()
                .bottom_0()
                .w(px(1.))
                .bg(theme.guide)
        }))
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
