// MongoDB 命令工作台页面：库级、跨集合的 shell 语句执行台。
//
// 形态对齐 Redis Workbench：顶部工具条（连接 / 库 + 运行 + 清空）+ 语句编辑器 +
// 结果区（一条语句一张卡片，可折叠 / 重跑 / 删除）。
// 与 `mongo_cli_content`（真实终端 REPL）并存：CLI 负责即敲即得，工作台负责可回看的执行记录。

/// 语句编辑器区高度：固定值，工作台不做分栏拖拽（CLI 已覆盖「要更大的输入区」这个诉求）。
const MONGO_WB_EDITOR_HEIGHT: f32 = 160.;
/// 单条记录正文的最大高度：超长结果在卡片内滚动，不把整张卡片撑高。
const MONGO_WB_RECORD_BODY_MAX_HEIGHT: f32 = 320.;

fn mongo_workbench_content(
    tab_id: TabId,
    workbench: &MongoWorkbenchState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let editor = this.mongo_workbench_state(tab_id, workbench, window, cx);
    let connection = connection_name(this.controller.state(), workbench.connection_id);
    let accent = this
        .controller
        .state()
        .connections
        .iter()
        .find(|connection| connection.config.id == workbench.connection_id)
        .map(|connection| connection_config_color_hex(&connection.config.options))
        .map(connection_color_rgba)
        .unwrap_or_else(|| connection_color_rgba(DEFAULT_CONNECTION_COLOR));

    div()
        .relative()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .child(mongo_workbench_toolbar(
            tab_id,
            workbench,
            connection,
            accent,
            colors,
            cx,
        ))
        .child(
            div()
                .h(px(MONGO_WB_EDITOR_HEIGHT))
                .min_h(px(0.))
                .flex_none()
                .border_b_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .debug_selector(|| "mongo-workbench-editor".to_string())
                .child(editor),
        )
        .child(mongo_workbench_results(tab_id, workbench, window, colors, cx))
}

fn mongo_workbench_toolbar(
    tab_id: TabId,
    workbench: &MongoWorkbenchState,
    connection: String,
    accent: gpui::Rgba,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let has_text = !workbench.text.trim().is_empty();
    let running = workbench.running;
    let has_results = !workbench.executions.is_empty();
    div()
        .h(px(40.))
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .bg(colors.panel_alt)
        .child(query_context_group(
            connection,
            workbench.database.clone(),
            accent,
            colors,
        ))
        .child(
            query_toolbar_icon_button(
                if running {
                    tr("app.content.running")
                } else {
                    tr("app.content.run")
                },
                AppIcon::Play,
                has_text && !running,
                running,
                rgb(0x20c76a),
                colors,
            )
            .debug_selector(|| "mongo-workbench-run".to_string())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if has_text && !running {
                        this.run_mongo_workbench(tab_id, None, cx);
                        cx.stop_propagation();
                    }
                }),
            ),
        )
        .child(
            query_toolbar_icon_button(
                tr("app.content.clear_results"),
                AppIcon::Trash,
                has_results && !running,
                false,
                rgb(0xff5c5c),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.dispatch(AppCommand::ClearMongoWorkbenchResults(tab_id), cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(div().flex_1())
        // 语法提示常驻右侧：工作台不支持 JS 求值，写错时用户要能立刻知道边界在哪。
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(tr("app.mongo.workbench.hint")),
        )
}

fn mongo_workbench_results(
    tab_id: TabId,
    workbench: &MongoWorkbenchState,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let body: gpui::AnyElement = if workbench.executions.is_empty() {
        if workbench.running {
            mongo_workbench_placeholder(tr("app.mongo.loading"), colors).into_any_element()
        } else if let Some(error) = workbench.error.as_ref() {
            mongo_workbench_placeholder(error.message.clone().into(), colors).into_any_element()
        } else {
            mongo_workbench_placeholder(tr("app.mongo.workbench.empty"), colors).into_any_element()
        }
    } else {
        let running = workbench.running;
        let mut list = div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .gap_2()
            .p_2();
        for execution in &workbench.executions {
            list = list.child(mongo_workbench_record_row(
                tab_id,
                execution,
                running,
                workbench.collapsed.contains(&execution.id),
                window,
                colors,
                cx,
            ));
        }
        list.into_any_element()
    };

    div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(colors.status_bg)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .flex()
                .flex_col()
                .child(body),
        )
}

fn mongo_workbench_placeholder(text: SharedString, colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_h(px(120.))
        .flex()
        .items_center()
        .justify_center()
        .p_4()
        .text_size(px(13.))
        .text_color(colors.muted)
        .child(text)
}

/// 单条执行记录卡片：头部（状态点 + 语句 + 时间 + 重跑 / 删除），身体是结果文本。
fn mongo_workbench_record_row(
    tab_id: TabId,
    execution: &MongoWorkbenchExecution,
    running: bool,
    collapsed: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let execution_id = execution.id;
    let success = execution.is_success();
    let status_color = if success {
        rgb(0x20c76a)
    } else {
        rgb(0xff5c5c)
    };
    let status_label = if success {
        tr("app.history.success")
    } else {
        tr("app.history.failed")
    };
    let time_label = redis_workbench_time_label(execution.started_at_unix_secs);
    let collapse_icon = if collapsed {
        AppIcon::ChevronRight
    } else {
        AppIcon::ChevronDown
    };

    let header = div()
        .id(("mongo-wb-record-header", execution_id))
        .h(px(34.))
        .px_2()
        .gap_2()
        .bg(colors.panel_alt)
        .cursor_pointer()
        .flex()
        .items_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if running {
                    return;
                }
                this.dispatch(
                    AppCommand::ToggleMongoWorkbenchRecordCollapse {
                        tab_id,
                        execution_id,
                    },
                    cx,
                );
                cx.stop_propagation();
            }),
        )
        .child(app_icon_box(collapse_icon, 24., 12., colors.muted).flex_none())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .text_size(px(12.))
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_color(colors.text)
                .child(execution.text.clone()),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(status_label)
                .child(time_label),
        )
        .child(mongo_workbench_record_action(
            execution_id,
            tr("app.mongo.workbench.rerun"),
            AppIcon::Play,
            rgb(0x20c76a),
            colors,
            !running,
            cx.listener(move |this, _, _, cx| {
                this.run_mongo_workbench(tab_id, Some(execution_id), cx);
                cx.stop_propagation();
            }),
        ))
        .child(mongo_workbench_record_action(
            execution_id,
            tr("app.common.delete"),
            AppIcon::Trash,
            rgb(0xff5c5c),
            colors,
            !running,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(
                    AppCommand::DeleteMongoWorkbenchRecord {
                        tab_id,
                        execution_id,
                    },
                    cx,
                );
                cx.stop_propagation();
            }),
        ));

    let mut card = div()
        .debug_selector(move || format!("mongo-workbench-record:{execution_id}"))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(header);
    if !collapsed {
        // 正文滚动：用原生 `overflow_y_scroll` + 显式 `vertical_scrollbar`，
        // 不用 `Scrollable` 封装 —— 后者的根节点是 `size_full()`，塞进 auto 高度的卡片里
        // 会把自身压成内容高度，`max_h` 之外的溢出既没有可滚范围也不画滚动条（实测长结果整段被裁）。
        let scroll_handle = window.use_keyed_state(
            ("mongo-wb-record-scroll", execution_id),
            cx,
            |_, _| ScrollHandle::new(),
        );
        let scroll_handle = scroll_handle.read(cx).clone();
        let body: gpui::AnyElement = match (&execution.output, &execution.error) {
            (_, Some(error)) => div()
                .text_color(status_color)
                .child(error.message.clone())
                .into_any_element(),
            (Some(output), None) => div()
                .child(mongo_workbench_styled_text(output, colors))
                .into_any_element(),
            (None, None) => div().into_any_element(),
        };
        card = card.child(
            div()
                .id(("mongo-wb-record-body", execution_id))
                .debug_selector(move || format!("mongo-workbench-record-body:{execution_id}"))
                .flex_none()
                .max_h(px(MONGO_WB_RECORD_BODY_MAX_HEIGHT))
                .border_t_1()
                .border_color(colors.border_soft)
                .relative()
                .overflow_y_scroll()
                .track_scroll(&scroll_handle)
                .vertical_scrollbar(&scroll_handle)
                .text_size(px(12.))
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_color(colors.text)
                .child(
                    div()
                        .debug_selector(move || {
                            format!("mongo-workbench-record-body-content:{execution_id}")
                        })
                        .p_2()
                        .child(body),
                ),
        );
    }
    card
}

fn mongo_workbench_record_action(
    execution_id: u64,
    label: SharedString,
    icon: AppIcon,
    icon_color: gpui::Rgba,
    colors: UiColors,
    enabled: bool,
    handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let tooltip_label = label.clone();
    div()
        .id(SharedString::from(format!(
            "mongo-wb-action:{execution_id}:{label}"
        )))
        .size(px(18.))
        .flex_none()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .justify_center()
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .tooltip(move |window, cx| {
                    Tooltip::new(tooltip_label.clone()).build(window, cx)
                })
                .on_mouse_down(MouseButton::Left, handler)
        })
        .child(app_icon(
            icon,
            14.,
            if enabled { icon_color } else { colors.muted },
        ))
}

/// 去掉 shell 输出里的 ANSI 颜色序列，并把 SGR 颜色转成富文本着色区间。
///
/// 连接器按终端排版输出（键蓝 / 字符串绿 / 数字橙 / 布尔青 / null 灰）。工作台是富文本面板，
/// 保留转义符会直接显示成 `[34m` 之类的垃圾，所以这里解析成 `StyledText` 的 run：
/// 文本去掉转义，颜色换成主题色（与 JSON 查看器同一套 `JsonEditorTheme` 色板）。
fn mongo_workbench_styled_text(text: &str, colors: UiColors) -> gpui::StyledText {
    let (plain, segments) = mongo_workbench_color_segments(text, JsonEditorTheme::from_colors(colors));
    let font = gpui::font(crate::PLATFORM_MONOSPACE_FONT);
    let runs = segments
        .into_iter()
        .map(|(len, color)| TextRun {
            len,
            font: font.clone(),
            color: color.unwrap_or(colors.text).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        })
        .collect();
    gpui::StyledText::new(plain).with_runs(runs)
}

/// 把带 SGR 的文本切成「纯文本 + 着色段」：`(字节长度, 颜色)`，`None` 表示默认正文色。
///
/// 与渲染分离，是为了让切段规则能直接单测（`StyledText` 不暴露文本与 run）。
fn mongo_workbench_color_segments(
    text: &str,
    theme: JsonEditorTheme,
) -> (String, Vec<(usize, Option<gpui::Rgba>)>) {
    let mut plain = String::with_capacity(text.len());
    let mut segments: Vec<(usize, Option<gpui::Rgba>)> = Vec::new();
    let mut current: Option<gpui::Rgba> = None;
    // 当前着色段在 `plain` 中的起点：颜色一变就把 [segment_start, len) 收成一段。
    let mut segment_start = 0usize;

    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            plain.push(ch);
            continue;
        }
        // CSI：ESC `[` … 终止字节落在 0x40..=0x7E。只认 `m`（SGR），其余序列直接丢弃。
        if chars.next() != Some('[') {
            continue;
        }
        let mut params = String::new();
        let mut final_byte = None;
        for next in chars.by_ref() {
            if ('\x40'..='\x7e').contains(&next) {
                final_byte = Some(next);
                break;
            }
            params.push(next);
        }
        if final_byte != Some('m') {
            continue;
        }
        let color = match params.as_str() {
            "0" | "" => None,
            "34" => Some(theme.key),
            "32" => Some(theme.string),
            "33" => Some(theme.number),
            "36" => Some(theme.boolean),
            // null 用中性灰（与标点同色）。
            "90" => Some(theme.punctuation),
            // 未知 SGR 不影响正文：按当前颜色继续。
            _ => current,
        };
        if color == current {
            continue;
        }
        if plain.len() > segment_start {
            segments.push((plain.len() - segment_start, current));
        }
        segment_start = plain.len();
        current = color;
    }
    if plain.len() > segment_start {
        segments.push((plain.len() - segment_start, current));
    }
    (plain, segments)
}

#[cfg(test)]
mod mongo_workbench_tests {
    use super::mongo_workbench_color_segments;
    use crate::JsonEditorTheme;
    use crate::ui_colors;

    fn segments(text: &str) -> (String, Vec<usize>) {
        let theme = JsonEditorTheme::from_colors(ui_colors(gpui_component::ThemeMode::Light));
        let (plain, segments) = mongo_workbench_color_segments(text, theme);
        (plain, segments.into_iter().map(|(len, _)| len).collect())
    }

    #[test]
    fn sgr_sequences_are_stripped_and_content_kept() {
        let text = "\x1b[34m_id\x1b[0m: \x1b[32m'abc'\x1b[0m\n";
        assert_eq!(segments(text).0, "_id: 'abc'\n");
    }

    #[test]
    fn runs_split_at_each_color_change() {
        // `_id`(蓝) `: `(默认) `'abc'`(绿) `\n`(默认)
        let text = "\x1b[34m_id\x1b[0m: \x1b[32m'abc'\x1b[0m\n";
        assert_eq!(segments(text).1, vec![3, 2, 5, 1]);
    }

    #[test]
    fn plain_text_becomes_a_single_default_run() {
        let (plain, lens) = segments("a > b");
        assert_eq!(plain, "a > b");
        assert_eq!(lens, vec![5]);
    }

    #[test]
    fn adjacent_same_color_runs_merge() {
        // 同一颜色连写两段（`\x1b[33m1\x1b[33m2`）不应切碎成两段。
        let (plain, lens) = segments("\x1b[33m12\x1b[0m");
        assert_eq!(plain, "12");
        assert_eq!(lens, vec![2]);
    }
}
