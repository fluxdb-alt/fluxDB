fn compare_transfer_hash(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}


fn compare_transfer_modal(
    state: &CompareTransferState,
    inputs: &CompareTransferInputs,
    connections: &[ConnectionConfig],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let source_name = connections
        .iter()
        .find(|connection| connection.id == state.source_connection_id)
        .map(|connection| connection.name.clone())
        .unwrap_or_default();
    let body = match state.tab {
        CompareTransferTab::Structure => {
            compare_structure_body(state, inputs, connections, &source_name, colors, cx)
        }
        CompareTransferTab::Data => {
            compare_data_transfer_body(state, inputs, connections, &source_name, colors, cx)
        }
    };
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .debug_selector(|| "compare-transfer-modal".into())
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.close_compare_transfer_modal(cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .w(px(1160.))
                // 数据页左栏字段最多，800 高给底部主按钮留出安全余量。
                .h(px(800.))
                .flex()
                .flex_col()
                .overflow_hidden()
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .text_color(colors.text)
                .key_context("CompareTransferModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.close_compare_transfer_modal(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(compare_transfer_header(state, colors, cx))
                .child(body)
                .child(compare_transfer_footer(state, colors, cx)),
        )
}

fn compare_transfer_header(
    state: &CompareTransferState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_none()
        .h(px(58.))
        .px_5()
        .flex()
        .items_center()
        .gap_4()
        .border_b_1()
        .border_color(colors.border_soft)
        .child(
            div()
                .text_size(ui_text_size(15.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(tr("app.compare_transfer.title")),
        )
        .child(div().flex_1())
        .child(compare_transfer_tab_button(
            CompareTransferTab::Structure,
            tr("app.compare_transfer.tab_structure"),
            state.tab,
            colors,
            cx,
        ))
        .child(compare_transfer_tab_button(
            CompareTransferTab::Data,
            tr("app.compare_transfer.tab_data"),
            state.tab,
            colors,
            cx,
        ))
}

fn compare_transfer_tab_button(
    tab: CompareTransferTab,
    label: SharedString,
    active: CompareTransferTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let selected = tab == active;
    div()
        .debug_selector(move || {
            format!(
                "compare-transfer-tab:{}",
                match tab {
                    CompareTransferTab::Structure => "structure",
                    CompareTransferTab::Data => "data",
                }
            )
        })
        .id(("compare-transfer-tab", tab as u8 as u64))
        .px_3()
        .py_1()
        .rounded(colors.radius)
        .text_size(ui_text_size(12.))
        .font_weight(if selected {
            gpui::FontWeight::SEMIBOLD
        } else {
            gpui::FontWeight::NORMAL
        })
        .text_color(if selected { colors.text } else { colors.muted })
        .bg(if selected {
            colors.tree_selected
        } else {
            colors.hover
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_compare_transfer_tab(tab, cx);
        }))
        .child(label)
}

fn compare_structure_body(
    state: &CompareTransferState,
    inputs: &CompareTransferInputs,
    connections: &[ConnectionConfig],
    source_name: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let target_name = compare_transfer_target_name(state, connections);
    // 左栏只放“连接与范围”，右侧整块留给结果；输入与结果分区后不必在同一个
    // 纵向流里同时找表单按钮和差异列表。
    let rail = compare_transfer_rail(colors)
        .child(
            compare_transfer_card(colors)
                .child(compare_transfer_card_header(
                    tr("app.compare_transfer.source"),
                    Some(source_name.to_string()),
                    colors,
                ))
                .child(compare_transfer_stack_field(
                    tr("app.compare_transfer.database"),
                    &inputs.source_database,
                    tr("app.compare_transfer.database_placeholder"),
                    colors,
                ))
                .child(compare_transfer_stack_field(
                    tr("app.compare_transfer.schema"),
                    &inputs.source_schema,
                    tr("app.compare_transfer.schema_placeholder"),
                    colors,
                )),
        )
        .child(
            compare_transfer_card(colors)
                .child(compare_transfer_card_header(
                    tr("app.compare_transfer.target"),
                    target_name,
                    colors,
                ))
                .child(compare_transfer_connection_field(
                    tr("app.compare_transfer.target_connection"),
                    &inputs.target_connection,
                    tr("app.compare_transfer.target_connection_placeholder"),
                    colors,
                ))
                .child(compare_transfer_stack_field(
                    tr("app.compare_transfer.database"),
                    &inputs.target_database,
                    tr("app.compare_transfer.database_placeholder"),
                    colors,
                ))
                .child(compare_transfer_stack_field(
                    tr("app.compare_transfer.schema"),
                    &inputs.target_schema,
                    tr("app.compare_transfer.schema_placeholder"),
                    colors,
                )),
        )
        // 主操作固定在左栏底部，滚动结果列表时按钮位置不漂移。
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .debug_selector(|| "compare-transfer-compare".into())
                        .child(
                            Button::new("compare-transfer-run")
                                .label(if state.compare_running {
                                    tr("app.compare_transfer.comparing")
                                } else {
                                    tr("app.compare_transfer.compare")
                                })
                                .disabled(state.compare_running || state.sync_running)
                                .w_full()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.start_compare_structure(cx);
                                })),
                        ),
                )
                .when_some(state.compare_error.clone(), |this, error| {
                    this.child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.danger)
                            .child(error),
                    )
                }),
        );

    let mut workspace = div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .gap_3()
        .p_4();
    if let Some(result) = &state.compare {
        workspace = workspace
            .child(compare_transfer_summary(result, state, colors, cx))
            .child(compare_transfer_result_list(result, state, colors, cx))
            .child(compare_transfer_ddl_preview(result, state, colors));
        if state.sync_running || !state.transfer_log.is_empty() {
            // 日志与结果同屏时固定高度，避免把列表和 DDL 一起压扁。
            workspace = workspace.child(
                div()
                    .flex_none()
                    .h(px(150.))
                    .child(compare_transfer_log(state, colors)),
            );
        }
    } else {
        workspace = workspace.child(compare_transfer_empty_state(colors));
        if state.sync_running || !state.transfer_log.is_empty() {
            workspace = workspace.child(
                div()
                    .flex_none()
                    .h(px(150.))
                    .child(compare_transfer_log(state, colors)),
            );
        }
    }
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .child(rail)
        .child(workspace)
}

/// 未对比时的空状态：说明「先选两端、再点开始对比」，避免整片留白。
fn compare_transfer_empty_state(colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_h(px(120.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_alt)
        .child(
            div()
                .text_size(ui_text_size(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(tr("app.compare_transfer.empty_title")),
        )
}

fn compare_transfer_target_name(
    state: &CompareTransferState,
    connections: &[ConnectionConfig],
) -> Option<String> {
    state
        .target_connection_id
        .and_then(|id| connections.iter().find(|connection| connection.id == id))
        .map(|connection| connection.name.clone())
}

/// 弹框左侧固定宽度的表单栏：上下两张端卡片 + 底部主操作。
fn compare_transfer_rail(colors: UiColors) -> Div {
    div()
        .debug_selector(|| "compare-transfer-rail".into())
        .w(px(300.))
        .flex_none()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .border_r_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_alt)
}

fn compare_transfer_card(colors: UiColors) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
}

fn compare_transfer_card_header(
    label: SharedString,
    name: Option<String>,
    colors: UiColors,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(20.))
        .child(compare_transfer_endpoint_tag(label, colors))
        .children(name.map(|name| {
            div()
                .min_w(px(0.))
                .truncate()
                .text_size(ui_text_size(12.))
                .text_color(colors.muted)
                .child(name)
        }))
}

fn compare_transfer_endpoint_tag(label: SharedString, colors: UiColors) -> Div {
    div()
        .flex_none()
        .px_2()
        .py_0p5()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_alt)
        .text_size(ui_text_size(10.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(label)
}

/// 左栏字段：标签在上、下拉占满栏宽，相比并排两列更容易逐项扫读。
fn compare_transfer_stack_field(
    label: impl Into<SharedString>,
    select: &Entity<SelectState<SearchableVec<String>>>,
    placeholder: impl Into<SharedString>,
    colors: UiColors,
) -> Div {
    compare_transfer_stack_select(label, select, placeholder, colors, true)
}

/// 目标连接不允许清空：清空后底层仍指向旧连接，容易误对比。
fn compare_transfer_connection_field(
    label: impl Into<SharedString>,
    select: &Entity<SelectState<SearchableVec<String>>>,
    placeholder: impl Into<SharedString>,
    colors: UiColors,
) -> Div {
    compare_transfer_stack_select(label, select, placeholder, colors, false)
}

fn compare_transfer_stack_select(
    label: impl Into<SharedString>,
    select: &Entity<SelectState<SearchableVec<String>>>,
    placeholder: impl Into<SharedString>,
    colors: UiColors,
    cleanable: bool,
) -> Div {
    let label = label.into();
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .child(label),
        )
        .child(
            Select::new(select)
                .placeholder(placeholder)
                .cleanable(cleanable)
                .w_full()
                .h(px(30.))
                .menu_width(px(260.)),
        )
}

fn compare_transfer_summary(
    result: &StructureCompareResult,
    state: &CompareTransferState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let all = result
        .tables
        .iter()
        .filter(|table| table.status != StructureDiffStatus::Same)
        .count();
    // 选中表里至少要有一张被当前动作选项覆盖，否则后台只会全量「跳过」。
    // 后台把空选择视为全部表，因此未选任何行时同样不允许同步。
    let has_actionable_selection = !state.selected_tables.is_empty()
        && result.tables.iter().any(|table| {
            state.selected_tables.contains(&table.key)
                && match table.status {
                    StructureDiffStatus::Added => state.sync_added,
                    StructureDiffStatus::Modified => state.recreate_modified,
                    StructureDiffStatus::Removed => state.drop_removed,
                    StructureDiffStatus::Same => false,
                }
        });
    // 右侧结果区比整宽弹框窄，统计与同步选项拆成两行，避免挤成一条溢出。
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(
                            tr("app.compare_transfer.result_count")
                                .replace("%{count}", &all.to_string()),
                        ),
                )
                .child(compare_transfer_status_text(
                    StructureDiffStatus::Added,
                    result.count(StructureDiffStatus::Added),
                    colors,
                ))
                .child(compare_transfer_status_text(
                    StructureDiffStatus::Modified,
                    result.count(StructureDiffStatus::Modified),
                    colors,
                ))
                .child(compare_transfer_status_text(
                    StructureDiffStatus::Removed,
                    result.count(StructureDiffStatus::Removed),
                    colors,
                ))
                .child(div().flex_1())
                .child(
                    Button::new("compare-transfer-toggle-all")
                        .label(tr("app.compare_transfer.toggle_all"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_compare_transfer_all(cx);
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Checkbox::new("compare-transfer-sync-added")
                        .checked(state.sync_added)
                        .label(format!(
                            "{} ({})",
                            tr("app.compare_transfer.sync_added"),
                            result.count(StructureDiffStatus::Added)
                        ))
                        .on_click(cx.listener(|this, checked, _, cx| {
                            if let Some(state) = this.compare_transfer.as_mut() {
                                state.sync_added = *checked;
                                cx.notify();
                            }
                        })),
                )
                .child(
                    Checkbox::new("compare-transfer-recreate")
                        .checked(state.recreate_modified)
                        .label(format!(
                            "{} ({})",
                            tr("app.compare_transfer.recreate_modified"),
                            result.count(StructureDiffStatus::Modified)
                        ))
                        .on_click(cx.listener(|this, checked, _, cx| {
                            if let Some(state) = this.compare_transfer.as_mut() {
                                state.recreate_modified = *checked;
                                cx.notify();
                            }
                        })),
                )
                .child(
                    Checkbox::new("compare-transfer-drop")
                        .checked(state.drop_removed)
                        .label(format!(
                            "{} ({})",
                            tr("app.compare_transfer.drop_removed"),
                            result.count(StructureDiffStatus::Removed)
                        ))
                        .on_click(cx.listener(|this, checked, _, cx| {
                            if let Some(state) = this.compare_transfer.as_mut() {
                                state.drop_removed = *checked;
                                cx.notify();
                            }
                        })),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .debug_selector(|| "compare-transfer-sync".into())
                        .child(
                            Button::new("compare-transfer-sync")
                                .label(if state.sync_running {
                                    tr("app.compare_transfer.syncing")
                                } else {
                                    tr("app.compare_transfer.sync")
                                })
                                .disabled(
                                    state.sync_running
                                        || state.compare_running
                                        || !has_actionable_selection,
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.start_sync_structure(cx);
                                })),
                        ),
                ),
        )
}

fn compare_transfer_status_text(
    status: StructureDiffStatus,
    count: usize,
    colors: UiColors,
) -> Div {
    let (label, color) = match status {
        StructureDiffStatus::Added => (tr("app.compare_transfer.status_added"), colors.success),
        StructureDiffStatus::Removed => (tr("app.compare_transfer.status_removed"), colors.warning),
        StructureDiffStatus::Modified => (tr("app.compare_transfer.status_modified"), colors.info),
        StructureDiffStatus::Same => (tr("app.compare_transfer.status_same"), colors.muted),
    };
    div()
        .text_size(ui_text_size(11.))
        .text_color(color)
        .child(format!("{label} {count}"))
}

fn compare_transfer_result_list(
    result: &StructureCompareResult,
    state: &CompareTransferState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .debug_selector(|| "compare-transfer-results".into())
        .id("compare-transfer-results")
        .flex_1()
        .min_h(px(96.))
        .flex()
        .flex_col()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .overflow_hidden()
        .child(
            div()
                .flex_none()
                .h(px(26.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .bg(colors.panel_alt)
                .border_b_1()
                .border_color(colors.border_soft)
                .text_size(ui_text_size(10.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(div().w(px(16.)))
                .child(
                    div()
                        .flex_1()
                        .child(tr("app.compare_transfer.column_table")),
                )
                .child(tr("app.compare_transfer.column_status")),
        )
        .child(div().flex_1().min_h(px(0.)).overflow_y_scrollbar().children(result.tables.iter().filter(|table| {
            table.status != StructureDiffStatus::Same
        }).map(|table| {
            let key = table.key.clone();
            let click_key = key.clone();
            let check_key = key.clone();
            let checked = state.selected_tables.contains(&table.key);
            let selected = state.selected_table.as_deref() == Some(table.key.as_str());
            let (status_label, status_color) = match table.status {
                StructureDiffStatus::Added => {
                    (tr("app.compare_transfer.status_added"), colors.success)
                }
                StructureDiffStatus::Removed => {
                    (tr("app.compare_transfer.status_removed"), colors.warning)
                }
                _ => (tr("app.compare_transfer.status_modified"), colors.info),
            };
            div()
                .id(("compare-transfer-row", compare_transfer_hash(&key)))
                .debug_selector(move || format!("compare-transfer-row:{key}"))
                .h(px(30.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(colors.border_soft)
                .bg(if selected { colors.tree_selected } else { colors.panel_bg })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_compare_transfer_table(click_key.clone(), cx);
                }))
                .child(
                    Checkbox::new(("compare-transfer-check", compare_transfer_hash(&table.key)))
                        .checked(checked)
                        .on_click(cx.listener(move |this, _checked, _, cx| {
                            this.toggle_compare_transfer_table(check_key.clone(), cx);
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(ui_text_size(12.))
                        .child(table.key.clone()),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(status_color)
                        .child(status_label),
                )
        })))
}

fn compare_transfer_ddl_preview(
    result: &StructureCompareResult,
    state: &CompareTransferState,
    colors: UiColors,
) -> Div {
    let Some(table) = state
        .selected_table
        .as_ref()
        .and_then(|key| result.tables.iter().find(|table| &table.key == key))
    else {
        return div();
    };
    div()
        .flex_1()
        .min_h(px(140.))
        .flex()
        .gap_2()
        .child(compare_transfer_ddl_pane(
            tr("app.compare_transfer.source_ddl"),
            table.source_ddl.as_deref().unwrap_or("--"),
            colors,
        ))
        .child(compare_transfer_ddl_pane(
            tr("app.compare_transfer.target_ddl"),
            table.target_ddl.as_deref().unwrap_or("--"),
            colors,
        ))
}

fn compare_transfer_ddl_pane(label: SharedString, ddl: &str, colors: UiColors) -> Div {
    let scroll_id = format!("compare-transfer-ddl-scroll:{label}");
    div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .overflow_hidden()
        .debug_selector(|| "compare-transfer-ddl-pane".into())
        .child(
            div()
                .flex_none()
                .h(px(26.))
                .px_2()
                .flex()
                .items_center()
                .text_size(ui_text_size(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                // 源/目标两个面板由同一处调用创建；不显式指定 id 会共用 Scrollable
                // 的调用位置 id，从而共用滚动状态。
                .id(scroll_id)
                .p_2()
                .text_size(ui_text_size(11.))
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .debug_selector(|| "compare-transfer-ddl-content".into())
                .child(ddl.to_string()),
        )
}

fn compare_transfer_log(state: &CompareTransferState, colors: UiColors) -> impl IntoElement {
    div()
        // 占满剩余高度，避免日志框下方留出大块空白。
        .flex_1()
        .min_h(px(120.))
        .flex()
        .flex_col()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .overflow_hidden()
        .child(
            div()
                .flex_none()
                .h(px(26.))
                .px_3()
                .flex()
                .items_center()
                .bg(colors.panel_alt)
                .border_b_1()
                .border_color(colors.border_soft)
                .text_size(ui_text_size(10.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(tr("app.compare_transfer.log_title")),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .flex()
                .flex_col()
                .gap_0p5()
                .bg(colors.input_bg)
                .p_3()
                .text_size(ui_text_size(11.))
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_color(colors.muted)
                .children(state.transfer_error.as_ref().map(|error| {
                    div().text_color(colors.danger).child(error.clone())
                }))
                .children(state.transfer_log.iter().map(|line| {
                    div().child(line.clone())
                })),
        )
}

fn compare_data_transfer_body(
    state: &CompareTransferState,
    inputs: &CompareTransferInputs,
    connections: &[ConnectionConfig],
    source_name: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let target_name = compare_transfer_target_name(state, connections);
    // 与结构对比共用左栏表单节奏：源表跟源端、目标表跟目标端，避免表名和连接分家。
    let rail = compare_transfer_rail(colors)
        .child(
            compare_transfer_card(colors)
                .child(compare_transfer_card_header(
                    tr("app.compare_transfer.source"),
                    Some(source_name.to_string()),
                    colors,
                ))
                .child(compare_transfer_stack_field(
                    tr("app.compare_transfer.database"),
                    &inputs.source_database,
                    tr("app.compare_transfer.database_placeholder"),
                    colors,
                ))
                .child(compare_transfer_stack_field(
                    tr("app.compare_transfer.schema"),
                    &inputs.source_schema,
                    tr("app.compare_transfer.schema_placeholder"),
                    colors,
                ))
                .child(compare_transfer_stack_field(
                    tr("app.compare_transfer.source_table"),
                    &inputs.source_table,
                    tr("app.compare_transfer.table_placeholder"),
                    colors,
                )),
        )
        .child(
            compare_transfer_card(colors)
                .child(compare_transfer_card_header(
                    tr("app.compare_transfer.target"),
                    target_name,
                    colors,
                ))
                .child(compare_transfer_connection_field(
                    tr("app.compare_transfer.target_connection"),
                    &inputs.target_connection,
                    tr("app.compare_transfer.target_connection_placeholder"),
                    colors,
                ))
                .child(compare_transfer_stack_field(
                    tr("app.compare_transfer.database"),
                    &inputs.target_database,
                    tr("app.compare_transfer.database_placeholder"),
                    colors,
                ))
                .child(compare_transfer_stack_field(
                    tr("app.compare_transfer.schema"),
                    &inputs.target_schema,
                    tr("app.compare_transfer.schema_placeholder"),
                    colors,
                ))
                .child(compare_transfer_stack_field(
                    tr("app.compare_transfer.target_table"),
                    &inputs.target_table,
                    tr("app.compare_transfer.target_table_placeholder"),
                    colors,
                )),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(ui_text_size(10.))
                        .text_color(colors.muted)
                        .child(tr("app.compare_transfer.write_mode")),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(compare_transfer_mode_button(
                            DataTransferMode::Append,
                            tr("app.compare_transfer.mode_append"),
                            state.transfer_mode,
                            colors,
                            cx,
                        ))
                        .child(compare_transfer_mode_button(
                            DataTransferMode::Overwrite,
                            tr("app.compare_transfer.mode_overwrite"),
                            state.transfer_mode,
                            colors,
                            cx,
                        )),
                )
                .child(
                    Checkbox::new("compare-transfer-create-target")
                        .checked(state.transfer_create_target)
                        .label(tr("app.compare_transfer.create_target"))
                        .on_click(cx.listener(|this, checked, _, cx| {
                            if let Some(state) = this.compare_transfer.as_mut() {
                                state.transfer_create_target = *checked;
                                cx.notify();
                            }
                        })),
                )
                .child(
                    div()
                        .debug_selector(|| "compare-transfer-start".into())
                        .child(
                            Button::new("compare-transfer-start")
                                .label(if state.transfer_running {
                                    tr("app.compare_transfer.transferring")
                                } else {
                                    tr("app.compare_transfer.start_transfer")
                                })
                                .disabled(state.transfer_running)
                                .w_full()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.start_data_transfer(cx);
                                })),
                        ),
                ),
        );

    // 右侧只放说明与运行日志；错误沿用日志区，不额外插一条孤立错误文本。
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .child(rail)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(compare_transfer_log(state, colors)),
        )
}

fn compare_transfer_mode_button(
    mode: DataTransferMode,
    label: SharedString,
    active: DataTransferMode,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let selected = mode == active;
    div()
        .id((
            "compare-transfer-mode",
            if mode == DataTransferMode::Append { 0u64 } else { 1u64 },
        ))
        .flex_1()
        .h(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(colors.radius)
        .border_1()
        .border_color(if selected { colors.info } else { colors.border })
        .bg(if selected { colors.tree_selected } else { colors.panel_bg })
        .text_size(ui_text_size(11.))
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(state) = this.compare_transfer.as_mut() {
                state.transfer_mode = mode;
                cx.notify();
            }
        }))
        .child(label)
}

fn compare_transfer_footer(
    state: &CompareTransferState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_none()
        .h(px(54.))
        .px_5()
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .border_t_1()
        .border_color(colors.border_soft)
        .when(
            state.sync_running || state.transfer_running,
            |this| {
                this.child(
                    Button::new("compare-transfer-cancel-task")
                        .label(tr("app.compare_transfer.cancel_task"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_compare_transfer_task(cx);
                        })),
                )
            },
        )
        .child(
            Button::new("compare-transfer-close")
                .label(tr("app.compare_transfer.close"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.close_compare_transfer_modal(cx);
                })),
        )
        .when(
            state.compare_running || state.sync_running || state.transfer_running,
            |this| {
                this.child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(tr("app.compare_transfer.background_hint")),
                )
            },
        )
}

#[cfg(test)]
mod compare_transfer_ddl_scroll_tests {
    use super::*;

    const LONG_DDL: &str = "CREATE TABLE \"public\".\"cmp_orders\" (\n  \"id\" bigint NOT NULL,\n  \"customer_id\" bigint NOT NULL,\n  \"amount\" numeric(12,2) NOT NULL,\n  \"status\" text NOT NULL,\n  \"note\" text,\n  \"created_at\" timestamptz NOT NULL,\n  \"updated_at\" timestamptz NOT NULL,\n  CONSTRAINT \"cmp_orders_pkey\" PRIMARY KEY (id),\n  CONSTRAINT \"cmp_orders_customer_fk\" FOREIGN KEY (customer_id) REFERENCES public.cmp_customers(id)\n);\nCREATE INDEX \"cmp_orders_status_idx\" ON public.cmp_orders (status);\n";

    struct DdlPanes {
        colors: UiColors,
    }

    impl gpui::Render for DdlPanes {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            div()
                .flex()
                .w(px(800.))
                .h(px(200.))
                .children((0..2).map(|index| {
                    // 同一调用点创建两个面板，复现源/目标共用一个 Scrollable id 的情况。
                    compare_transfer_ddl_pane(format!("pane{index}").into(), LONG_DDL, self.colors)
                }))
        }
    }

    fn redraw(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
    }

    #[gpui::test]
    fn ddl_panes_scroll(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
        });
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|_| DdlPanes {
                colors: ui_colors(gpui_component::ThemeMode::Light),
            });
            Root::new(view, window, cx)
        });
        redraw(cx);
        let before = cx
            .debug_bounds("compare-transfer-ddl-content")
            .expect("ddl content");
        let pane = cx
            .debug_bounds("compare-transfer-ddl-pane")
            .expect("ddl pane");
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: pane.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-80.))),
            ..Default::default()
        });
        redraw(cx);
        let after = cx
            .debug_bounds("compare-transfer-ddl-content")
            .expect("ddl content after scroll");
        assert!(
            after.origin.y < before.origin.y,
            "DDL 内容应随滚轮上移：before={:?} after={:?}",
            before.origin,
            after.origin
        );
    }
}
