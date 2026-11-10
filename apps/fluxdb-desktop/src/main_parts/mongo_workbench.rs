// MongoDB 命令工作台页面：库级、跨集合的 shell 语句执行台。
//
// 形态对齐 Redis Workbench：顶部工具条（连接 / 库 + 运行 + 清空）+ 语句编辑器 +
// 结果区（一条语句一张卡片，可折叠 / 重跑 / 删除）。
// 与 `mongo_cli_content`（真实终端 REPL）并存：CLI 负责即敲即得，工作台负责可回看的执行记录。

/// 顶部工具条高度，用于估算上下分栏的可用高度。
const MONGO_WB_TOOLBAR_HEIGHT: f32 = 40.;
/// 单条记录正文的最大高度：超长结果在卡片内滚动，不把整张卡片撑高。
const MONGO_WB_RECORD_BODY_MAX_HEIGHT: f32 = 320.;
/// 卡片正文最多参与排版的字节数。
///
/// 连接器已经把单条结果压到 8 MiB，但 `StyledText` 会为整段文本做字形整形，
/// 8 MiB 一次就能让 UI 卡住；超过此预算只渲染前缀，完整内容走「复制」。
///
/// 卡片正文最高只有 320px，64 KiB（约一千行）足够在卡片内滚动浏览，
/// 再多也只会增加不可见的排版成本。
const MONGO_WB_RECORD_PREVIEW_BYTES: usize = 64 * 1024;

/// 截取卡片正文的可渲染前缀，返回 `(前缀, 是否被预览截断)`。
///
/// 切点落在字符边界上；靠近末尾有换行时退到换行处，避免渲染出半行。
fn mongo_workbench_preview(text: &str) -> (&str, bool) {
    if text.len() <= MONGO_WB_RECORD_PREVIEW_BYTES {
        return (text, false);
    }
    let mut end = MONGO_WB_RECORD_PREVIEW_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if let Some(line_end) = text[..end].rfind('\n').filter(|line_end| *line_end > 0) {
        end = line_end;
    }
    (&text[..end], true)
}

/// 结果大小标签（1024 进制，保留一位小数）。
fn mongo_workbench_bytes_label(bytes: usize) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// 上下分栏可用高度估算：视口高度扣除工具条（与 Redis Workbench 同一模型）。
fn mongo_workbench_split_height(window: &Window) -> f32 {
    (f32::from(window.viewport_size().height) - MONGO_WB_TOOLBAR_HEIGHT).max(0.)
}

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
    // 编辑器高度来自持久化占比（与 Redis 同一套夹取规则：上下都有像素下限）。
    let ratio =
        f32::from(this.controller.state().settings.mongo_workbench_editor_ratio) / 100.0;
    let editor_size = workbench_editor_height(ratio, mongo_workbench_split_height(window));

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
                .h(px(editor_size))
                .min_h(px(WORKBENCH_EDITOR_MIN_HEIGHT))
                // 允许收缩：按视口估算的高度可能大于实际分栏空间，
                // 不收缩会把下方结果区压到最小高度以下。
                .flex_initial()
                .border_b_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .debug_selector(|| "mongo-workbench-editor".to_string())
                .child(editor),
        )
        .child(workbench_split_handle(
            WorkbenchSplitTarget::Mongo,
            editor_size,
            ResultsPlacement::Bottom,
            colors,
            cx,
        ))
        .child(mongo_workbench_results(
            tab_id, workbench, this, window, colors, cx,
        ))
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
        .h(px(MONGO_WB_TOOLBAR_HEIGHT))
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
    this: &NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let body: gpui::AnyElement = if workbench.executions.is_empty() {
        if workbench.running {
            center_loading_message(tr("app.mongo.loading"), colors).into_any_element()
        } else if let Some(error) = workbench.error.as_ref() {
            mongo_workbench_placeholder(error.message.clone().into(), colors).into_any_element()
        } else {
            mongo_workbench_placeholder(tr("app.mongo.workbench.empty"), colors).into_any_element()
        }
    } else {
        let mut list = div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .gap_2()
            .p_2();
        for execution in &workbench.executions {
            list = list.child(mongo_workbench_record_row(
                tab_id, execution, workbench, this, window, colors, cx,
            ));
        }
        list.into_any_element()
    };

    div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(WORKBENCH_RESULT_MIN_HEIGHT))
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
    workbench: &MongoWorkbenchState,
    this: &NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let execution_id = execution.id;
    let running = workbench.running;
    let collapsed = workbench.collapsed.contains(&execution_id);
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
    // 卡片头直接给出「几条 / 多大 / 多久」：没有这些信息，用户无法判断结果是否完整。
    let mut meta_parts = Vec::new();
    if let Some(count) = execution.document_count {
        meta_parts.push(
            tr("app.mongo.workbench.document_count")
                .replace("%{count}", &count.to_string()),
        );
    }
    if let Some(output) = execution.output.as_ref() {
        meta_parts.push(mongo_workbench_bytes_label(output.len()));
    }
    if execution.elapsed_ms > 0 {
        meta_parts.push(
            tr("app.mongo.workbench.elapsed")
                .replace("%{ms}", &execution.elapsed_ms.to_string()),
        );
    }
    let meta_label = meta_parts.join(" · ");
    let truncated_label = execution
        .truncated
        .then(|| tr("app.mongo.workbench.truncated").to_string());
    let has_output = execution
        .output
        .as_ref()
        .is_some_and(|output| !output.is_empty());
    // 文档解析结果按记录缓存在窗口状态里：记录落地后不可变，折叠 / 展开只重建元素。
    // 键带上标签页：不同工作台的记录 ID 各自从 1 开始，只用执行 ID 会读到别的工作台的文档。
    let document_page = if execution.documents.is_empty() {
        None
    } else {
        let data = window.use_keyed_state(
            SharedString::from(format!(
                "mongo-wb-record-docs:{}:{execution_id}",
                tab_id.0
            )),
            cx,
            |_, _| Rc::new(MongoPageData::from_documents(execution.documents.clone())),
        );
        Some(data.read(cx).clone())
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
                // 元信息变长后必须允许收缩：否则窄窗口会把右侧的重跑 / 删除按钮
                // 顶出可视区，点击被 gpui 静默丢弃。
                .flex_initial()
                .min_w(px(0.))
                .overflow_hidden()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(status_label)
                .when(!meta_label.is_empty(), |this| this.child(meta_label))
                .when_some(truncated_label, |this, label| {
                    this.child(
                        div()
                            .text_color(rgb(0xffb020))
                            .child(label),
                    )
                })
                .child(time_label),
        )
        .child(mongo_workbench_record_action(
            execution_id,
            tr("app.common.copy"),
            AppIcon::Copy,
            colors.text,
            colors,
            has_output,
            cx.listener(move |this, _, _, cx| {
                // 点击时才按 id 取完整结果：结果可能有数 MB，不能每帧 clone 一份进闭包。
                let output = this
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
                    .and_then(|tab| match &tab.kind {
                        TabKind::MongoWorkbench(workbench) => workbench
                            .executions
                            .iter()
                            .find(|record| record.id == execution_id)
                            .and_then(|record| record.output.clone()),
                        _ => None,
                    });
                if let Some(output) = output {
                    // 复制去掉 ANSI 的完整结果：卡片正文可能只展示了前缀。
                    let (plain, _) = mongo_workbench_color_segments(
                        &output,
                        JsonEditorTheme::from_colors(colors),
                    );
                    cx.write_to_clipboard(ClipboardItem::new_string(plain));
                }
                cx.stop_propagation();
            }),
        ))
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
        // 键带上标签页：窗口级 keyed state 不按标签页隔离，两个工作台各有记录 1 时
        // 光用 `execution_id` 会共用同一个滚动句柄。
        let scroll_handle = window.use_keyed_state(
            SharedString::from(format!("mongo-wb-record-scroll:{}:{execution_id}", tab_id.0)),
            cx,
            |_, _| ScrollHandle::new(),
        );
        let scroll_handle = scroll_handle.read(cx).clone();
        let body: gpui::AnyElement = match (&execution.output, &execution.error) {
            (_, Some(error)) => div()
                .text_color(status_color)
                .child(error.message.clone())
                .into_any_element(),
            // 文档结果优先用字段卡片展示；文本仍保留在记录里供复制。
            (_, None) if document_page.is_some() => {
                let page = document_page.clone().expect("文档结果已缓存");
                // 选择行序接上前面记录：同一页多条记录时，每条记录从 0 开始会让行序重复。
                let documents_before = workbench
                    .executions
                    .iter()
                    .take_while(|candidate| candidate.id != execution_id)
                    .map(|candidate| candidate.documents.len() as u64)
                    .sum::<u64>();
                mongo_workbench_document_cards(
                    tab_id,
                    execution,
                    page,
                    documents_before,
                    this,
                    colors,
                    cx,
                )
                .into_any_element()
            }
            (Some(output), None) if output.is_empty() => div()
                .text_color(colors.muted)
                .child(tr("app.mongo.workbench.no_output"))
                .into_any_element(),
            (Some(output), None) => {
                let (preview, previewed) = mongo_workbench_preview(output);
                let styled = div().child(mongo_workbench_styled_text(preview, colors));
                if previewed {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(0xffb020))
                                .child(
                                    tr("app.mongo.workbench.preview_truncated")
                                        .replace(
                                            "%{preview}",
                                            &mongo_workbench_bytes_label(
                                                MONGO_WB_RECORD_PREVIEW_BYTES,
                                            ),
                                        )
                                        .replace(
                                            "%{size}",
                                            &mongo_workbench_bytes_label(output.len()),
                                        ),
                                ),
                        )
                        .child(styled)
                        .into_any_element()
                } else {
                    styled.into_any_element()
                }
            }
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

/// 一条记录内的文档卡片列表；`page` 是该记录的解析快照，由调用方按记录缓存。
fn mongo_workbench_document_cards(
    tab_id: TabId,
    execution: &MongoWorkbenchExecution,
    page: Rc<MongoPageData>,
    documents_before: u64,
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let execution_id = execution.id;
    let sets = |map: &BTreeMap<TabId, BTreeSet<String>>| {
        Rc::new(map.get(&tab_id).cloned().unwrap_or_default())
    };
    let view = MongoDocumentView {
        tab_id,
        collapsed_documents: sets(&this.mongo_collapsed_documents),
        expanded_fields: sets(&this.mongo_expanded_fields),
        json_fold_toggles: sets(&this.mongo_json_fold_toggles),
        page,
        document_order_base: documents_before,
        colors,
    };
    let mut list = div()
        .p_2()
        .flex()
        .flex_col()
        .gap_2()
        .debug_selector(move || format!("mongo-workbench-documents:{execution_id}"));
    for (index, document) in execution.documents.iter().enumerate() {
        // 折叠 / JSON 展开键加记录前缀：不同记录里同 `_id` 的文档各管各的状态。
        let document_key = format!(
            "wb:{execution_id}:{}",
            mongo_document_key(document, index)
        );
        list = list.child(mongo_workbench_document_card(
            &view,
            execution_id,
            index,
            document,
            &document_key,
            cx,
        ));
    }
    list
}

/// 工作台里的一张文档卡片：折叠箭头 + 序号 + `_id` + 视图切换 / 复制，
/// 正文按当前视图显示字段列表或 JSON。
fn mongo_workbench_document_card(
    view: &MongoDocumentView,
    execution_id: u64,
    index: usize,
    document: &fluxdb_core::MongoDocument,
    document_key: &str,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let tab_id = view.tab_id;
    let colors = view.colors;
    let collapsed = view.collapsed_documents.contains(document_key);
    let toggle_key = document_key.to_string();
    // 视图模式收在字段展开集合里；字段路径用 `/` 分隔，控制符不会与真实字段名冲突。
    let json_key = format!("{document_key}\u{1e}json");
    let json_mode = view.expanded_fields.contains(&json_key);
    let value = view.value(index);
    let id_scope = format!("mongo-wb-json:{execution_id}");

    let header = div()
        .h(px(28.))
        .px_1p5()
        .flex()
        .items_center()
        .gap_1p5()
        .border_b_1()
        .border_color(colors.border_soft)
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.toggle_mongo_document_collapsed(tab_id, toggle_key.clone(), cx);
                cx.stop_propagation();
            }),
        )
        .child(mongo_card_icon_button(
            format!("mongo-wb-doc-toggle:{execution_id}:{index}"),
            format!("mongo-workbench-doc-toggle:{execution_id}:{index}"),
            if collapsed {
                AppIcon::ChevronRight
            } else {
                AppIcon::ChevronDown
            },
            if collapsed {
                tr("app.mongo.expand_document")
            } else {
                tr("app.mongo.collapse_document")
            },
            colors.muted,
            true,
            false,
            colors,
        ))
        .child(
            div()
                .pr_2()
                .border_r_1()
                .border_color(colors.border_soft)
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(SharedString::from(format!("#{}", index + 1))),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_size(px(11.))
                .text_color(colors.text)
                .child(SharedString::from(format!(
                    "{}: {}",
                    tr("app.mongo.table.id"),
                    document.id
                ))),
        )
        // 视图切换：文档 ↔ JSON。按钮显示的是「切过去之后」的视图。
        .child({
            let toggle_key = json_key.clone();
            mongo_card_icon_button(
                format!("mongo-wb-doc-view:{execution_id}:{index}"),
                format!("mongo-workbench-doc-view:{execution_id}:{index}"),
                if json_mode {
                    AppIcon::List
                } else {
                    AppIcon::FileSql
                },
                if json_mode {
                    tr("app.mongo.display.documents")
                } else {
                    tr("app.mongo.display.json")
                },
                colors.text,
                true,
                false,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.toggle_mongo_field(tab_id, toggle_key.clone(), cx);
                    cx.stop_propagation();
                }),
            )
        })
        .child(
            mongo_card_icon_button(
                format!("mongo-wb-doc-copy:{execution_id}:{index}"),
                format!("mongo-workbench-doc-copy:{execution_id}:{index}"),
                AppIcon::Copy,
                tr("app.common.copy"),
                colors.text,
                true,
                false,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    // 点击时才按记录 / 下标取原文：整批文档可能有数 MB，不能每帧 clone。
                    let json = this
                        .controller
                        .state()
                        .tabs
                        .iter()
                        .find(|tab| tab.id == tab_id)
                        .and_then(|tab| match &tab.kind {
                            TabKind::MongoWorkbench(workbench) => workbench
                                .executions
                                .iter()
                                .find(|record| record.id == execution_id)
                                .and_then(|record| record.documents.get(index))
                                .map(|document| document.json.clone()),
                            _ => None,
                        });
                    if let Some(json) = json {
                        cx.write_to_clipboard(ClipboardItem::new_string(json));
                        this.show_message(
                            tr("app.mongo.copy_done").as_ref(),
                            AppMessageKind::Success,
                            cx,
                        );
                    }
                    cx.stop_propagation();
                }),
            ),
        );

    let body = if collapsed {
        div()
    } else if json_mode {
        mongo_json_block(
            view,
            index,
            document,
            value,
            document_key,
            &id_scope,
            cx,
        )
    } else {
        mongo_field_list(view, index, document, value, document_key, &id_scope, cx)
    };

    div()
        .debug_selector(move || format!("mongo-workbench-document:{execution_id}:{index}"))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_alt)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(header)
        .child(body)
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
    use super::{
        MONGO_WB_RECORD_PREVIEW_BYTES, mongo_workbench_bytes_label,
        mongo_workbench_color_segments, mongo_workbench_preview,
    };
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

    #[test]
    fn preview_caps_huge_output_at_a_line_boundary() {
        let text = format!("{}\n{}", "a".repeat(MONGO_WB_RECORD_PREVIEW_BYTES), "tail");
        let (preview, truncated) = mongo_workbench_preview(&text);
        assert!(truncated);
        assert!(preview.len() <= MONGO_WB_RECORD_PREVIEW_BYTES);
        assert!(!preview.ends_with('\n'), "退到换行处时不要保留换行本身");
        assert!(!preview.contains("tail"), "超预算的尾部不能参与渲染");

        let small = "a > b";
        assert_eq!(mongo_workbench_preview(small), (small, false));
    }

    #[test]
    fn preview_is_utf8_safe() {
        // 切点正好落在多字节字符中间时必须回退到字符边界。
        let text = "中".repeat(MONGO_WB_RECORD_PREVIEW_BYTES / 3 + 1);
        let (preview, truncated) = mongo_workbench_preview(&text);
        assert!(truncated);
        assert!(text.starts_with(preview));
    }

    #[test]
    fn byte_labels_stay_readable() {
        assert_eq!(mongo_workbench_bytes_label(512), "512 B");
        assert_eq!(mongo_workbench_bytes_label(1024), "1.0 KB");
        assert_eq!(mongo_workbench_bytes_label(1024 * 1024 * 3 / 2), "1.5 MB");
    }
}
