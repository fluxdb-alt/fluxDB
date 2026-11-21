// 同步向导界面：四步（选择对象 / 表配对 / 同步选项 / 差异预览）。
//
// 只渲染状态，不直接调用控制器；所有动作通过 NavicatMain 上的方法与后台交互。

#[allow(clippy::too_many_arguments)]
fn sync_wizard_modal(
    state: &SyncWizardState,
    pair_rows: &[SyncPairRow],
    field_map: Option<&SyncFieldMapState>,
    type_map: Option<&SyncTypeMapState>,
    inputs: &SyncInputs,
    connections: &[ConnectionConfig],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .debug_selector(|| "sync-wizard-modal".into())
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        // 点遮罩不关闭：向导里有已填配置与预览结果，误点外面就丢太贵。
        // 只允许「取消」或右上角关闭按钮退出（Esc 等同取消）。
        .child(
            div()
                .w(px(1120.))
                .h(px(760.))
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
                .key_context(state.mode.context_key())
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.close_sync_wizard(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(sync_header(state, connections, colors, cx))
                .child(sync_stepper(state, colors))
                .child(match state.step {
                    1 => sync_step_pick(state, inputs, connections, colors, cx).into_any_element(),
                    2 => sync_step_pair(state, pair_rows, colors, cx).into_any_element(),
                    3 => sync_step_options(state, inputs, colors, cx).into_any_element(),
                    _ => sync_step_preview(state, colors, cx).into_any_element(),
                })
                .child(sync_footer(state, colors, cx))
                .when_some(field_map, |this, field_map| {
                    this.child(sync_field_map_dialog(field_map, colors, cx))
                })
                .when_some(type_map, |this, type_map| {
                    this.child(sync_type_map_dialog(type_map, colors, cx))
                }),
        )
}

fn sync_header(
    state: &SyncWizardState,
    connections: &[ConnectionConfig],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let source_name = sync_connection_name(connections, state.source_connection_id);
    let target_name = state
        .target_connection_id
        .map(|id| sync_connection_name(connections, id))
        .unwrap_or_default();
    div()
        .flex_none()
        .h(px(62.))
        .px_5()
        .flex()
        .items_center()
        .gap_3()
        .border_b_1()
        .border_color(colors.border_soft)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(ui_text_size(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(match state.mode {
                            SyncMode::Structure => tr("app.sync.title_structure"),
                            SyncMode::Data => tr("app.sync.title_data"),
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(ui_text_size(10.5))
                        .text_color(colors.muted)
                        .child(source_name)
                        .child("→")
                        .child(target_name),
                ),
        )
        .child(div().flex_1())
        .child(sync_mode_tab(
            SyncMode::Structure,
            tr("app.sync.mode_structure"),
            state.mode,
            colors,
            cx,
        ))
        .child(sync_mode_tab(
            SyncMode::Data,
            tr("app.sync.mode_data"),
            state.mode,
            colors,
            cx,
        ))
        .child(
            div()
                .debug_selector(|| "sync-wizard-close-icon".into())
                .child(
                    Button::new("sync-wizard-close-icon")
                        .label("✕")
                        .on_click(cx.listener(|this, _, _, cx| this.close_sync_wizard(cx))),
                ),
        )
}

fn sync_mode_tab(
    mode: SyncMode,
    label: SharedString,
    active: SyncMode,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let selected = mode == active;
    div()
        .debug_selector(move || {
            format!(
                "sync-mode-tab:{}",
                match mode {
                    SyncMode::Structure => "structure",
                    SyncMode::Data => "data",
                }
            )
        })
        .id(("sync-mode-tab", match mode {
            SyncMode::Structure => 0u64,
            SyncMode::Data => 1u64,
        }))
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
            if let Some(state) = this.sync_wizard.as_mut()
                && state.mode != mode
                && !state.running
            {
                state.mode = mode;
                state.step = 1;
                state.selected_table = None;
                state.error = None;
            }
            cx.notify();
        }))
        .child(label)
}

fn sync_stepper(state: &SyncWizardState, colors: UiColors) -> Div {
    let labels = [
        tr("app.sync.step_pick"),
        tr("app.sync.step_pair"),
        tr("app.sync.step_options"),
        tr("app.sync.step_preview"),
    ];
    let mut row = div()
        .flex_none()
        .h(px(48.))
        .px_5()
        .flex()
        .items_center()
        .gap_3()
        .border_b_1()
        .border_color(colors.border_soft);
    for (index, label) in labels.into_iter().enumerate() {
        let number = index as u8 + 1;
        let active = state.step == number;
        let done = state.step > number;
        if index > 0 {
            row = row.child(
                div()
                    .flex_1()
                    .h(px(1.))
                    .bg(colors.border_soft),
            );
        }
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(ui_text_size(11.5))
                .text_color(if active {
                    colors.text
                } else if done {
                    colors.info
                } else {
                    colors.muted
                })
                .child(
                    div()
                        .w(px(22.))
                        .h(px(22.))
                        .rounded(px(11.))
                        .border_1()
                        .border_color(if active || done {
                            colors.info
                        } else {
                            colors.border
                        })
                        .bg(if active || done {
                            colors.info
                        } else {
                            colors.panel_bg
                        })
                        .text_color(if active || done { colors.panel_bg } else { colors.muted })
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(number.to_string()),
                )
                .child(label),
        );
    }
    row
}

fn sync_step_pick(
    state: &SyncWizardState,
    inputs: &SyncInputs,
    connections: &[ConnectionConfig],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 搜索词只过滤列表展示；空列表时「全选」作用于全部表，与未搜索时一致。
    let query = normalized_sidebar_search(&state.table_filter);
    let visible: Vec<&SyncTableInfo> = state
        .tables
        .iter()
        .filter(|table| {
            search_matches_text(&table.name, &query)
                || table
                    .schema
                    .as_deref()
                    .is_some_and(|schema| search_matches_text(schema, &query))
        })
        .collect();
    let visible_keys: Vec<String> = visible.iter().map(|table| table.key.clone()).collect();
    let has_match = !visible.is_empty();
    let all_visible_selected =
        has_match && visible.iter().all(|table| state.selected_tables.contains(&table.key));
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .gap_4()
        .p_4()
        .child(
            div()
                .debug_selector(|| "sync-pick-rail".into())
                .w(px(272.))
                .flex_none()
                .flex()
                .flex_col()
                .gap_2()
                .child(sync_endpoint_card(
                    true,
                    state,
                    inputs,
                    connections,
                    colors,
                ))
                // 源 → 目标的方向提示，占据两卡片之间的视觉分隔。
                .child(
                    div()
                        .flex_none()
                        .h(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .w(px(22.))
                                .h(px(22.))
                                .rounded(px(11.))
                                .border_1()
                                .border_color(colors.border)
                                .bg(colors.panel_bg)
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child("⇅"),
                        ),
                )
                .child(sync_endpoint_card(
                    false,
                    state,
                    inputs,
                    connections,
                    colors,
                ))
                .child(
                    div()
                        .mt_1()
                        .px_3()
                        .py_2()
                        .rounded(colors.radius)
                        .bg(colors.panel_alt)
                        .text_size(ui_text_size(10.5))
                        .text_color(colors.muted)
                        .child(tr("app.sync.pick_note")),
                ),
        )
        .child(
            div()
                .debug_selector(|| "sync-table-panel".into())
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .child(
                    div()
                        .flex_none()
                        .h(px(40.))
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_3()
                        .border_b_1()
                        .border_color(colors.border_soft)
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.sync.tables")),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(10.5))
                                .text_color(colors.muted)
                                .child(sync_table_count_label(state)),
                        )
                        // 表清单搜索：只过滤展示，不影响已选表与同步请求。
                        .child(
                            div()
                                .debug_selector(|| "sync-table-search".into())
                                .track_focus(&inputs.table_filter.read(cx).focus_handle(cx))
                                .flex_1()
                                .min_w(px(120.))
                                .max_w(px(320.))
                                .h(px(28.))
                                .px_2()
                                .rounded(colors.radius)
                                .border_1()
                                .border_color(colors.border)
                                .hover(move |style| style.border_color(colors.muted))
                                .focus(move |style| {
                                    style.border_color(if colors.is_dark {
                                        rgb(0x8ab4ff)
                                    } else {
                                        rgb(0x111111)
                                    })
                                })
                                .bg(colors.input_bg)
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(app_icon(AppIcon::Search, 14., colors.muted))
                                .child(
                                    div().flex_1().min_w(px(0.)).child(
                                        Input::new(&inputs.table_filter)
                                            .appearance(false)
                                            .focus_bordered(false)
                                            .cleanable(true)
                                            .w_full()
                                            .h_full()
                                            .text_size(ui_text_size(12.)),
                                    ),
                                ),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .debug_selector(|| "sync-select-all".into())
                                .child(
                                    Button::new("sync-select-all")
                                        .label(if all_visible_selected {
                                            tr("app.sync.select_none")
                                        } else {
                                            tr("app.sync.select_all")
                                        })
                                        .disabled(visible.is_empty())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toggle_sync_tables(visible_keys.clone(), cx);
                                        })),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scrollbar()
                        .children(visible.into_iter().map(|table| {
                            sync_table_row(
                                table,
                                state.selected_tables.contains(&table.key),
                                colors,
                                cx,
                            )
                        })),
                )
                .when(state.tables.is_empty(), |this| {
                    this.child(
                        div()
                            .p_4()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.muted)
                            .child(if state.tables_loading {
                                tr("app.sync.tables_loading")
                            } else {
                                tr("app.sync.tables_empty")
                            }),
                    )
                })
                .when(!state.tables.is_empty() && !state.tables_loading && !has_match, |this| {
                    this.child(
                        div()
                            .p_4()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.muted)
                            .child(tr("app.sync.tables_no_match")),
                    )
                })
                .child(
                    div()
                        .debug_selector(|| "sync-selected-bar".into())
                        .flex_none()
                        .min_h(px(40.))
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .text_size(ui_text_size(10.5))
                        .text_color(colors.muted)
                        .child(
                            tr("app.sync.selected_count").replace(
                                "%{count}",
                                &state.selected_tables.len().to_string(),
                            ),
                        )
                        .child(div().flex_1())
                        .child(tr("app.sync.mapping_note")),
                ),
        )
}

fn sync_endpoint_card(
    source: bool,
    state: &SyncWizardState,
    inputs: &SyncInputs,
    connections: &[ConnectionConfig],
    colors: UiColors,
) -> Div {
    let connection_id = if source {
        Some(state.source_connection_id)
    } else {
        state.target_connection_id
    };
    let connection_name = connection_id
        .map(|id| sync_connection_name(connections, id))
        .unwrap_or_else(|| tr("app.sync.choose_target").to_string());
    let (database, schema) = if source {
        (&inputs.source_database, &inputs.source_schema)
    } else {
        (&inputs.target_database, &inputs.target_schema)
    };
    // MySQL 家族的 database 与 schema 同义，SQLite 没有 schema 维度；
    // 这些类型不渲染 Schema 选择器，避免用户误以为还要再选一层。
    let uses_schemas = connection_id
        .and_then(|id| connections.iter().find(|connection| connection.id == id))
        .is_some_and(|connection| connection.kind.uses_schemas());
    div()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .p_3()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded(colors.radius)
                        .text_size(ui_text_size(10.))
                        .bg(if source { colors.hover } else { colors.tree_selected })
                        .text_color(if source { colors.muted } else { colors.info })
                        .child(if source {
                            tr("app.sync.source")
                        } else {
                            tr("app.sync.target")
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(connection_name),
                ),
        )
        .when(!source, |this| {
            this.child(sync_select_field(
                tr("app.sync.target_connection"),
                &inputs.target_connection,
                colors,
                false,
            ))
        })
        .child(sync_select_field(
            tr("app.sync.database"),
            database,
            colors,
            true,
        ))
        .when(uses_schemas, |this| {
            this.child(
                sync_select_field(tr("app.sync.schema"), schema, colors, true)
                    .debug_selector(|| {
                        if source {
                            "sync-schema-source".into()
                        } else {
                            "sync-schema-target".into()
                        }
                    }),
            )
        })
}

fn sync_select_field(
    label: SharedString,
    select: &Entity<SelectState<SearchableVec<String>>>,
    colors: UiColors,
    cleanable: bool,
) -> Div {
    div()
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
                .cleanable(cleanable)
                .w_full()
                .h(px(30.))
                .menu_width(px(260.)),
        )
}

fn sync_table_row(
    table: &SyncTableInfo,
    selected: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let key = table.key.clone();
    let key_for_toggle = table.key.clone();
    div()
        .id(("sync-table-row", sync_wizard_hash(&key)))
        .px_3()
        .py_1p5()
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
            this.toggle_sync_table(key_for_toggle.clone(), cx);
            cx.stop_propagation();
        }))
        .child(
            Checkbox::new(("sync-table-check", sync_wizard_hash(&key))).checked(selected),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .child(table.name.clone()),
        )
        .when_some(table.schema.clone(), |this, schema| {
            this.child(
                div()
                    .text_size(ui_text_size(10.5))
                    .text_color(colors.muted)
                    .child(schema),
            )
        })
}

/// 表配对「目标表」列宽；表头与每行共用，保证上下对齐。
const PAIR_TARGET_WIDTH: f32 = 320.;
/// 表配对「操作」列宽（字段映射按钮），表头留同宽空位。
const PAIR_ACTION_WIDTH: f32 = 96.;
/// 字段映射弹框「目标字段」列宽；列头与每行共用。
const FIELD_MAP_TARGET_WIDTH: f32 = 280.;
/// 字段表「比对键」「跳过」勾选列宽；表头与每行共用。
const FIELD_TOGGLE_WIDTH: f32 = 56.;
/// 类型映射弹框「源类型」列宽。
const TYPE_MAP_SOURCE_WIDTH: f32 = 140.;
/// 类型映射弹框「目标类型」输入框列宽；表头与每行共用。
const TYPE_MAP_INPUT_WIDTH: f32 = 260.;

/// 第二步：表配对。整宽列表 + 虚拟滚动，表多时也只渲染视口内的行。
fn sync_step_pair(
    state: &SyncWizardState,
    pair_rows: &[SyncPairRow],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let row_count = pair_rows.len();
    let selected_count = state.selected_tables.len().to_string();
    let list = uniform_list(
        ("sync-pair-list", sync_wizard_hash(state.mode.context_key())),
        row_count,
        cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
            range
                .filter_map(|index| {
                    let row = this.sync_pair_rows.get(index)?;
                    let key = row.source_key.clone();
                    let selector_key = key.clone();
                    let key_for_map = key.clone();
                    // 数据同步给人字段映射/比对键；结构同步给单表类型映射覆盖。
                    let mode = this.sync_wizard.as_ref().map(|state| state.mode);
                    Some(
                        div()
                            .debug_selector(move || format!("sync-pair-row:{selector_key}"))
                            .h(px(40.))
                            .w_full()
                            .px_3()
                            .flex()
                            .items_center()
                            .gap_3()
                            .border_b_1()
                            .border_color(colors.border_soft)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_size(ui_text_size(11.))
                                    .child(key),
                            )
                            .child(
                                div()
                                    .debug_selector({
                                        let key = row.source_key.clone();
                                        move || format!("sync-pair-target:{key}")
                                    })
                                    .w(px(PAIR_TARGET_WIDTH))
                                    .child(
                                        Select::new(&row.select)
                                            .w(px(PAIR_TARGET_WIDTH))
                                            .h(px(28.))
                                            .menu_width(px(360.)),
                                    ),
                            )
                            // 操作列定宽，表头与每行才能对齐。
                            .child(
                                div().flex_none().w(px(PAIR_ACTION_WIDTH)).flex().justify_end().child(
                                    match mode {
                                        Some(SyncMode::Data) => Button::new((
                                            "sync-field-map",
                                            sync_wizard_hash(&key_for_map),
                                        ))
                                        .debug_selector({
                                            let key = key_for_map.clone();
                                            move || format!("sync-field-map:{key}")
                                        })
                                        .label(tr("app.sync.field_settings"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_sync_field_map(key_for_map.clone(), window, cx);
                                        }))
                                        .into_any_element(),
                                        Some(SyncMode::Structure) => Button::new((
                                            "sync-type-map",
                                            sync_wizard_hash(&key_for_map),
                                        ))
                                        .debug_selector({
                                            let key = key_for_map.clone();
                                            move || format!("sync-type-map:{key}")
                                        })
                                        .label(tr("app.sync.type_map"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_sync_type_map(key_for_map.clone(), window, cx);
                                        }))
                                        .into_any_element(),
                                        None => div().into_any_element(),
                                    },
                                ),
                            )
                            .into_any_element(),
                    )
                })
                .collect::<Vec<_>>()
        }),
    )
    .flex_1()
    .min_h(px(0.));
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(tr("app.sync.step_pair")),
                )
                .child(
                    div()
                        .text_size(ui_text_size(10.5))
                        .text_color(colors.muted)
                        .child(tr("app.sync.pair_desc")),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(ui_text_size(10.5))
                        .text_color(colors.muted)
                        .child(tr("app.sync.pair_count").replace("%{count}", &selected_count)),
                ),
        )
        .child(
            div()
                .debug_selector(|| "sync-pair-panel".into())
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .overflow_hidden()
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .child(
                    div()
                        .flex_none()
                        .h(px(32.))
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_3()
                        .border_b_1()
                        .border_color(colors.border_soft)
                        .text_size(ui_text_size(10.))
                        .text_color(colors.muted)
                        .child(div().flex_1().child(tr("app.sync.pair_source")))
                        .child(
                            div()
                                .debug_selector(|| "sync-pair-header-target".into())
                                .flex_none()
                                .w(px(PAIR_TARGET_WIDTH))
                                .child(tr("app.sync.pair_target")),
                        )
                        .child(div().flex_none().w(px(PAIR_ACTION_WIDTH))),
                )
                .when(row_count == 0, |this| {
                    this.child(
                        div()
                            .p_4()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.muted)
                            .child(tr("app.sync.pair_empty")),
                    )
                })
                .when(row_count > 0, |this| this.child(list)),
        )
}

/// 单表「字段设置」弹框：比对键、跳过字段、字段映射三段。
/// 保存后回写三份文本（比对键/跳过字段写表级项，映射只留改名项）。
fn sync_field_map_dialog(
    dialog: &SyncFieldMapState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let body = if dialog.loading {
        // 读字段期间与预览一致：居中转圈 + 文案。
        div()
            .py_8()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .text_size(ui_text_size(11.))
            .text_color(colors.muted)
            .child(sync_loading_spinner(14., colors.muted))
            .child(tr("app.sync.field_map_loading"))
            .into_any_element()
    } else if let Some(error) = dialog.error.clone() {
        div()
            .p_4()
            .text_size(ui_text_size(11.))
            .text_color(colors.danger)
            .child(error)
            .into_any_element()
    } else if dialog.rows.is_empty() {
        div()
            .p_4()
            .text_size(ui_text_size(11.))
            .text_color(colors.muted)
            .child(tr("app.sync.field_map_empty"))
            .into_any_element()
    } else {
        // 单表字段表：一行一列，同时给出映射下拉与「比对键 / 跳过」勾选，
        // 不再分三段列表，避免同一批列名重复铺三遍。
        let header_cell = |width: Option<f32>, label: SharedString| {
            let mut cell = div()
                .flex_none()
                .h_full()
                .flex()
                .items_center()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted);
            cell = match width {
                Some(width) => cell.w(px(width)),
                None => cell.flex_1(),
            };
            cell.child(label)
        };
        let header = div()
            .h(px(30.))
            .px_3()
            .flex()
            .items_center()
            .gap_3()
            .bg(colors.panel_alt)
            .border_b_1()
            .border_color(colors.border_soft)
            .child(header_cell(None, tr("app.sync.field_map_source")))
            .child(
                div()
                    .debug_selector(|| "sync-field-map-header-target".into())
                    .flex_none()
                    .w(px(FIELD_MAP_TARGET_WIDTH))
                    .h_full()
                    .flex()
                    .items_center()
                    .text_size(ui_text_size(10.))
                    .text_color(colors.muted)
                    .child(tr("app.sync.field_map_target")),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(FIELD_TOGGLE_WIDTH))
                    .h_full()
                    .flex()
                    .justify_center()
                    .items_center()
                    .text_size(ui_text_size(10.))
                    .text_color(colors.muted)
                    .child(tr("app.sync.field_key_short")),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(FIELD_TOGGLE_WIDTH))
                    .h_full()
                    .flex()
                    .justify_center()
                    .items_center()
                    .text_size(ui_text_size(10.))
                    .text_color(colors.muted)
                    .child(tr("app.sync.field_skip_short")),
            );
        let rows = dialog.rows.iter().map(|row| {
            let source = row.source.clone();
            // 勾选作用在当前映射到的目标列；映射为「不写入」时没有目标列，
            // 比对键/跳过都置灰，避免写入无效列名。
            let target = row
                .select
                .read(cx)
                .selected_value()
                .cloned()
                .filter(|value| value != &dialog.skip_label);
            let is_primary = target.as_deref().is_some_and(|target| {
                dialog
                    .target_columns
                    .iter()
                    .any(|(name, primary)| *primary && name == target)
            });
            let key_checked = target
                .as_deref()
                .is_some_and(|target| dialog.key_columns.contains(target));
            let skip_checked = target
                .as_deref()
                .is_some_and(|target| dialog.skip_columns.contains(target));
            let key_toggle = target.clone();
            let skip_toggle = target.clone();
            div()
                .debug_selector({
                    let source = source.clone();
                    move || format!("sync-field-map-row:{source}")
                })
                .h(px(38.))
                .px_3()
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .border_b_1()
                .border_color(colors.border_soft)
                .hover(|style| style.bg(colors.hover))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .debug_selector({
                                    let source = source.clone();
                                    move || format!("sync-field-map-source:{source}")
                                })
                                .min_w(px(0.))
                                .truncate()
                                .text_size(ui_text_size(11.))
                                .child(source.clone()),
                        )
                        .when(is_primary, |this| {
                            this.child(
                                div()
                                    .flex_none()
                                    .px(px(6.))
                                    .rounded(colors.radius)
                                    .bg(colors.panel_alt)
                                    .text_size(ui_text_size(9.))
                                    .text_color(colors.muted)
                                    .child(tr("app.sync.field_primary_key_tag")),
                            )
                        }),
                )
                // Select 根节点是 size_full，直接放进 flex 行会吃掉整行宽度，
                // 必须套一层定宽容器。
                .child(
                    div()
                        .flex_none()
                        .debug_selector({
                            let source = source.clone();
                            move || format!("sync-field-map-target:{source}")
                        })
                        .w(px(FIELD_MAP_TARGET_WIDTH))
                        .child(
                            Select::new(&row.select)
                                .w(px(FIELD_MAP_TARGET_WIDTH))
                                .h(px(28.))
                                .menu_width(px(FIELD_MAP_TARGET_WIDTH)),
                        ),
                )
                .child(
                    div()
                        .debug_selector({
                            let source = source.clone();
                            move || format!("sync-field-key:{source}")
                        })
                        .flex_none()
                        .w(px(FIELD_TOGGLE_WIDTH))
                        .flex()
                        .justify_center()
                        .child(
                            Checkbox::new(format!("sync-field-key:{}", source))
                                .checked(key_checked)
                                .disabled(key_toggle.is_none())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(target) = key_toggle.clone() {
                                        this.toggle_sync_field_key(target, cx);
                                    }
                                })),
                        ),
                )
                .child(
                    div()
                        .debug_selector({
                            let source = source.clone();
                            move || format!("sync-field-skip:{source}")
                        })
                        .flex_none()
                        .w(px(FIELD_TOGGLE_WIDTH))
                        .flex()
                        .justify_center()
                        .child(
                            Checkbox::new(format!("sync-field-skip:{}", source))
                                .checked(skip_checked)
                                .disabled(skip_toggle.is_none())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(target) = skip_toggle.clone() {
                                        this.toggle_sync_field_skip(target, cx);
                                    }
                                })),
                        ),
                )
        });
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(ui_text_size(10.))
                    .text_color(colors.muted)
                    .child(tr("app.sync.field_settings_hint")),
            )
            .child(
                div()
                    .w_full()
                    .rounded(colors.radius)
                    .border_1()
                    .border_color(colors.border_soft)
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(header)
                    .children(rows),
            )
            .into_any_element()
    };
    div()
        .debug_selector(|| "sync-field-map-overlay".into())
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(hsla(220. / 360., 0.12, 0.08, 0.46))
        .flex()
        .items_center()
        .justify_center()
        // 同主向导：点遮罩不关闭，避免误点丢掉未保存的字段映射。
        .child(
            div()
                .debug_selector(|| "sync-field-map-dialog".into())
                .w(px(720.))
                .max_h(px(560.))
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
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex_none()
                        .px_4()
                        .py_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_b_1()
                        .border_color(colors.border_soft)
                        .child(
                            div()
                                .text_size(ui_text_size(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.sync.field_settings")),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(10.5))
                                .text_color(colors.muted)
                                .child(format!("{} → {}", dialog.source_name, dialog.target_name)),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("sync-field-map-close")
                                .label("✕")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_sync_field_map(cx);
                                })),
                        ),
                )
                .child(
                    // 弹框高度跟随内容（外层只有 max_h），这里不能用 flex_1：
                    // 自动高度容器里 basis=0 会把字段列表压成 0 高，看起来像空弹框。
                    // 滚动用原生 overflow_y_scroll() + 同级滚动条：`overflow_y_scrollbar()`
                    // 的封装根节点是 size_full，在这种自动高度布局里滚不动。
                    div()
                        .relative()
                        .flex_none()
                        .w_full()
                        .child(
                            // 滚动能力（track_scroll / overflow_y_scroll）只在
                            // Stateful 元素上，必须先给滚动容器一个 id。
                            div()
                                .id("sync-field-map-list")
                                .flex_none()
                                .min_h(px(0.))
                                .max_h(px(440.))
                                .track_scroll(&dialog.scroll)
                                .overflow_y_scroll()
                                .p_3()
                                .child(body),
                        )
                        .vertical_scrollbar(&dialog.scroll),
                )
                .child(
                    div()
                        .flex_none()
                        .h(px(52.))
                        .px_4()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .child(
                            div()
                                .text_size(ui_text_size(10.5))
                                .text_color(colors.muted)
                                .child(tr("app.sync.field_map_hint")),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("sync-field-map-cancel")
                                .debug_selector(|| "sync-field-map-cancel".into())
                                .label(tr("app.sync.cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_sync_field_map(cx);
                                })),
                        )
                        .child(
                            Button::new("sync-field-map-save")
                                .debug_selector(|| "sync-field-map-save".into())
                                .label(tr("app.sync.field_map_save"))
                                .disabled(dialog.loading || dialog.error.is_some())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_sync_field_map(window, cx);
                                })),
                        ),
                ),
        )
}

/// 单表「类型映射」弹框（结构同步）：逐列填写目标类型，留空沿用自动映射。
fn sync_type_map_dialog(
    dialog: &SyncTypeMapState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let body = if dialog.loading {
        div()
            .py_8()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .text_size(ui_text_size(11.))
            .text_color(colors.muted)
            .child(sync_loading_spinner(14., colors.muted))
            .child(tr("app.sync.field_map_loading"))
            .into_any_element()
    } else if let Some(error) = dialog.error.clone() {
        div()
            .p_4()
            .text_size(ui_text_size(11.))
            .text_color(colors.danger)
            .child(error)
            .into_any_element()
    } else if dialog.rows.is_empty() {
        div()
            .p_4()
            .text_size(ui_text_size(11.))
            .text_color(colors.muted)
            .child(tr("app.sync.field_map_empty"))
            .into_any_element()
    } else {
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(30.))
                    .px_3()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_3()
                    .bg(colors.panel_alt)
                    .border_b_1()
                    .border_color(colors.border_soft)
                    .text_size(ui_text_size(10.))
                    .text_color(colors.muted)
                    .child(div().flex_1().child(tr("app.sync.field_map_source")))
                    .child(
                        div()
                            .flex_none()
                            .w(px(TYPE_MAP_SOURCE_WIDTH))
                            .child(tr("app.sync.type_map_source_type")),
                    )
                    .child(
                        div()
                            .debug_selector(|| "sync-type-map-header-target".into())
                            .flex_none()
                            .w(px(TYPE_MAP_INPUT_WIDTH))
                            .child(tr("app.sync.type_map_target_type")),
                    ),
            )
            .children(dialog.rows.iter().map(|row| {
                let column = row.column.clone();
                div()
                    .debug_selector({
                        let column = column.clone();
                        move || format!("sync-type-map-row:{column}")
                    })
                    .h(px(38.))
                    .px_3()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(colors.border_soft)
                    .hover(|style| style.bg(colors.hover))
                    .child(
                        div()
                            .debug_selector({
                                let column = column.clone();
                                move || format!("sync-type-map-column:{column}")
                            })
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(ui_text_size(11.))
                            .child(column.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(TYPE_MAP_SOURCE_WIDTH))
                            .truncate()
                            .text_size(ui_text_size(10.))
                            .text_color(colors.muted)
                            .child(row.source_type.clone()),
                    )
                    .child(
                        div()
                            .debug_selector({
                                let column = column.clone();
                                move || format!("sync-type-map-input:{column}")
                            })
                            .flex_none()
                            .w(px(TYPE_MAP_INPUT_WIDTH))
                            .child(Input::new(&row.input).w(px(TYPE_MAP_INPUT_WIDTH)).h(px(28.))),
                    )
            }))
            .into_any_element()
    };
    div()
        .debug_selector(|| "sync-type-map-overlay".into())
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(hsla(220. / 360., 0.12, 0.08, 0.46))
        .flex()
        .items_center()
        .justify_center()
        // 同主向导：点遮罩不关闭，避免误点丢掉未保存的类型映射。
        .child(
            div()
                .debug_selector(|| "sync-type-map-dialog".into())
                .w(px(720.))
                .max_h(px(560.))
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
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex_none()
                        .px_4()
                        .py_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_b_1()
                        .border_color(colors.border_soft)
                        .child(
                            div()
                                .text_size(ui_text_size(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.sync.type_map")),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(10.5))
                                .text_color(colors.muted)
                                .child(format!("{} → {}", dialog.source_name, dialog.target_name)),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("sync-type-map-close")
                                .label("✕")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_sync_type_map(cx);
                                })),
                        ),
                )
                .child(
                    // 与字段设置一致：自动高度里不能用 flex_1，滚动用原生滚动 + 同级条。
                    div()
                        .relative()
                        .flex_none()
                        .w_full()
                        .child(
                            div()
                                .id("sync-type-map-list")
                                .flex_none()
                                .min_h(px(0.))
                                .max_h(px(440.))
                                .track_scroll(&dialog.scroll)
                                .overflow_y_scroll()
                                .child(body),
                        )
                        .vertical_scrollbar(&dialog.scroll),
                )
                .child(
                    div()
                        .flex_none()
                        .h(px(52.))
                        .px_4()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .child(
                            div()
                                .text_size(ui_text_size(10.5))
                                .text_color(colors.muted)
                                .child(tr("app.sync.type_map_hint")),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("sync-type-map-cancel")
                                .debug_selector(|| "sync-type-map-cancel".into())
                                .label(tr("app.sync.cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_sync_type_map(cx);
                                })),
                        )
                        .child(
                            Button::new("sync-type-map-save")
                                .debug_selector(|| "sync-type-map-save".into())
                                .label(tr("app.sync.field_map_save"))
                                .disabled(dialog.loading || dialog.error.is_some())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.save_sync_type_map(cx);
                                })),
                        ),
                ),
        )
}

fn sync_step_options(
    state: &SyncWizardState,
    inputs: &SyncInputs,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .p_4()
        .flex()
        .justify_center()
        .child(
            div()
                .debug_selector(|| "sync-options-column".into())
                .w(px(720.))
                .flex()
                .flex_col()
                .gap_3()
                .when(state.mode == SyncMode::Data, |this| {
                    this.child(sync_option_card(
                        tr("app.sync.actions_card"),
                        tr("app.sync.data_actions_desc"),
                        colors,
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(sync_action_row(
                                "sync-action-insert",
                                tr("app.sync.action_insert"),
                                tr("app.sync.action_insert_hint"),
                                state.data_options.insert,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.data_options.insert = !state.data_options.insert;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-action-update",
                                tr("app.sync.action_update"),
                                tr("app.sync.action_update_hint"),
                                state.data_options.update,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.data_options.update = !state.data_options.update;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-action-delete",
                                tr("app.sync.action_delete"),
                                tr("app.sync.action_delete_hint"),
                                state.data_options.delete,
                                true,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.data_options.delete = !state.data_options.delete;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-action-overwrite",
                                tr("app.sync.action_overwrite"),
                                tr("app.sync.action_overwrite_hint"),
                                state.data_options.overwrite,
                                true,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.data_options.overwrite = !state.data_options.overwrite;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            )),
                    ))
                    // 冲突策略：默认跳过失败行，避免一行脏数据让整表停下来。
                    .child(sync_option_card(
                        tr("app.sync.conflict_card"),
                        tr("app.sync.conflict_desc"),
                        colors,
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(sync_action_row(
                                "sync-row-error-skip",
                                tr("app.sync.row_error_skip"),
                                tr("app.sync.row_error_skip_hint"),
                                state.data_options.row_error_policy == RowErrorPolicy::Skip,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.data_options.row_error_policy =
                                            RowErrorPolicy::Skip;
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-row-error-stop",
                                tr("app.sync.row_error_stop"),
                                tr("app.sync.row_error_stop_hint"),
                                state.data_options.row_error_policy == RowErrorPolicy::Stop,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.data_options.row_error_policy =
                                            RowErrorPolicy::Stop;
                                    }
                                    cx.notify();
                                }),
                            )),
                    ))
                    // 唯一键冲突：只改 INSERT 语句，不影响预览与比对结果。
                    .child(sync_option_card(
                        tr("app.sync.insert_conflict_card"),
                        tr("app.sync.insert_conflict_desc"),
                        colors,
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(sync_action_row(
                                "sync-insert-conflict-error",
                                tr("app.sync.insert_conflict_error"),
                                tr("app.sync.insert_conflict_error_hint"),
                                state.data_options.insert_conflict == InsertConflictMode::Error,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.data_options.insert_conflict =
                                            InsertConflictMode::Error;
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-insert-conflict-ignore",
                                tr("app.sync.insert_conflict_ignore"),
                                tr("app.sync.insert_conflict_ignore_hint"),
                                state.data_options.insert_conflict == InsertConflictMode::Ignore,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.data_options.insert_conflict =
                                            InsertConflictMode::Ignore;
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-insert-conflict-overwrite",
                                tr("app.sync.insert_conflict_overwrite"),
                                tr("app.sync.insert_conflict_overwrite_hint"),
                                state.data_options.insert_conflict
                                    == InsertConflictMode::Overwrite,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.data_options.insert_conflict =
                                            InsertConflictMode::Overwrite;
                                    }
                                    cx.notify();
                                }),
                            )),
                    ))
                    // 表级事务只作用于覆盖模式：清空与写入同属一个事务，失败整表回滚。
                    .child(sync_option_card(
                        tr("app.sync.atomic_card"),
                        tr("app.sync.atomic_desc"),
                        colors,
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(sync_action_row(
                                "sync-atomic-off",
                                tr("app.sync.atomic_off"),
                                tr("app.sync.atomic_off_hint"),
                                !state.data_options.atomic_table,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.data_options.atomic_table = false;
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-atomic-on",
                                tr("app.sync.atomic_on"),
                                tr("app.sync.atomic_on_hint"),
                                state.data_options.atomic_table,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.data_options.atomic_table = true;
                                    }
                                    cx.notify();
                                }),
                            )),
                    ))
                    // 导出迁移 SQL：同步照常执行，同时把成功语句落盘；后端拒绝与表级事务同用。
                    .child(sync_option_card(
                        tr("app.sync.export_sql_card"),
                        tr("app.sync.export_sql_desc"),
                        colors,
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(sync_action_row(
                                "sync-export-sql-choose",
                                tr("app.sync.export_sql_choose"),
                                state
                                    .data_options
                                    .export_sql_path
                                    .as_ref()
                                    .map(|path| path.display().to_string())
                                    .unwrap_or_else(|| {
                                        tr("app.sync.export_sql_choose_hint").to_string()
                                    })
                                    .into(),
                                state.data_options.export_sql_path.is_some(),
                                false,
                                colors,
                                cx.listener(|this, _, window, cx| {
                                    this.choose_sync_export_path(window, cx);
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-export-sql-off",
                                tr("app.sync.export_sql_off"),
                                tr("app.sync.export_sql_off_hint"),
                                state.data_options.export_sql_path.is_none(),
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        if state.data_options.export_sql_path.take().is_some() {
                                            state.clear_preview();
                                        }
                                    }
                                    cx.notify();
                                }),
                            )),
                    ))
                    // 「高级」默认收起：表级设置在第二步「字段设置」完成，
                    // 这里的批量文本只是多表场景的兜底。
                    .child(sync_advanced_card(
                        state.advanced_expanded,
                        tr("app.sync.advanced_desc"),
                        colors,
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(sync_form_row(
                                tr("app.sync.key_columns"),
                                Input::new(&inputs.key_columns).w(px(300.)).h(px(30.)),
                                tr("app.sync.key_columns_hint"),
                                colors,
                            ))
                            .child(sync_form_row(
                                tr("app.sync.column_map"),
                                Input::new(&inputs.column_map).w(px(300.)).h(px(30.)),
                                tr("app.sync.column_map_hint"),
                                colors,
                            ))
                            .child(sync_form_row(
                                tr("app.sync.skip_columns"),
                                Input::new(&inputs.skip_columns).w(px(300.)).h(px(30.)),
                                tr("app.sync.skip_columns_hint"),
                                colors,
                            ))
                            .child(sync_form_row(
                                tr("app.sync.batch_rows"),
                                Input::new(&inputs.batch_rows).w(px(120.)).h(px(30.)),
                                tr("app.sync.batch_rows_hint"),
                                colors,
                            ))
                            .child(sync_action_row(
                                "sync-action-create-missing",
                                tr("app.sync.action_create_missing"),
                                tr("app.sync.action_create_missing_hint"),
                                state.data_options.create_missing,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.data_options.create_missing =
                                            !state.data_options.create_missing;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            )),
                        cx,
                    ))
                })
                .when(state.mode == SyncMode::Structure, |this| {
                    this.child(sync_option_card(
                        tr("app.sync.actions_card"),
                        tr("app.sync.structure_actions_desc"),
                        colors,
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(sync_action_row(
                                "sync-structure-create-table",
                                tr("app.sync.action_create_table"),
                                tr("app.sync.action_create_table_hint"),
                                state.structure_options.create_table,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.structure_options.create_table =
                                            !state.structure_options.create_table;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-structure-add-column",
                                tr("app.sync.action_add_column"),
                                tr("app.sync.action_add_column_hint"),
                                state.structure_options.add_column,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.structure_options.add_column =
                                            !state.structure_options.add_column;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-structure-alter-column",
                                tr("app.sync.action_alter_column"),
                                tr("app.sync.action_alter_column_hint"),
                                state.structure_options.alter_column,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.structure_options.alter_column =
                                            !state.structure_options.alter_column;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-structure-add-index",
                                tr("app.sync.action_add_index"),
                                tr("app.sync.action_add_index_hint"),
                                state.structure_options.add_index,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.structure_options.add_index =
                                            !state.structure_options.add_index;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-structure-add-fk",
                                tr("app.sync.action_add_fk"),
                                tr("app.sync.action_add_fk_hint"),
                                state.structure_options.add_foreign_key,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.structure_options.add_foreign_key =
                                            !state.structure_options.add_foreign_key;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-structure-add-check",
                                tr("app.sync.action_add_check"),
                                tr("app.sync.action_add_check_hint"),
                                state.structure_options.add_check,
                                false,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.structure_options.add_check =
                                            !state.structure_options.add_check;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            )),
                    ))
                    .child(sync_option_card(
                        tr("app.sync.danger_card"),
                        tr("app.sync.danger_desc"),
                        colors,
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(sync_action_row(
                                "sync-structure-drop-column",
                                tr("app.sync.action_drop_column"),
                                tr("app.sync.action_drop_column_hint"),
                                state.structure_options.drop_column,
                                true,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.structure_options.drop_column =
                                            !state.structure_options.drop_column;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-structure-drop-index",
                                tr("app.sync.action_drop_index"),
                                tr("app.sync.action_drop_index_hint"),
                                state.structure_options.drop_index,
                                true,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.structure_options.drop_index =
                                            !state.structure_options.drop_index;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-structure-drop-fk",
                                tr("app.sync.action_drop_fk"),
                                tr("app.sync.action_drop_fk_hint"),
                                state.structure_options.drop_foreign_key,
                                true,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.structure_options.drop_foreign_key =
                                            !state.structure_options.drop_foreign_key;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-structure-drop-check",
                                tr("app.sync.action_drop_check"),
                                tr("app.sync.action_drop_check_hint"),
                                state.structure_options.drop_check,
                                true,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.structure_options.drop_check =
                                            !state.structure_options.drop_check;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(sync_action_row(
                                "sync-structure-drop-table",
                                tr("app.sync.action_drop_table"),
                                tr("app.sync.action_drop_table_hint"),
                                state.structure_options.drop_table,
                                true,
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.sync_wizard.as_mut() {
                                        state.structure_options.drop_table =
                                            !state.structure_options.drop_table;
                                        state.clear_preview();
                                    }
                                    cx.notify();
                                }),
                            )),
                    ))
                    // 与数据同步一致：批量文本收进「高级」，默认收起。
                    .child(sync_advanced_card(
                        state.advanced_expanded,
                        tr("app.sync.advanced_desc_structure"),
                        colors,
                        div().flex().flex_col().gap_2().child(sync_form_row(
                            tr("app.sync.type_overrides"),
                            Input::new(&inputs.type_overrides).w(px(300.)).h(px(30.)),
                            tr("app.sync.type_overrides_hint"),
                            colors,
                        )),
                        cx,
                    ))
                }),
        )
}

/// 「高级」折叠卡：表级设置之外的批量文本/杂项，默认收起。
fn sync_advanced_card(
    expanded: bool,
    description: SharedString,
    colors: UiColors,
    body: Div,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .debug_selector(|| "sync-advanced-card".into())
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .child(
            div()
                .id("sync-advanced-toggle")
                .debug_selector(|| "sync-advanced-toggle".into())
                .px_4()
                .py_3()
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .child(app_icon(
                    if expanded {
                        AppIcon::ChevronDown
                    } else {
                        AppIcon::ChevronRight
                    },
                    14.,
                    colors.muted,
                ))
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(tr("app.sync.advanced_card")),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(ui_text_size(10.5))
                        .text_color(colors.muted)
                        .child(description),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(state) = this.sync_wizard.as_mut() {
                        state.advanced_expanded = !state.advanced_expanded;
                    }
                    cx.notify();
                })),
        )
        .when(expanded, |this| {
            this.child(div().px_4().pb_4().flex().flex_col().gap_2().child(body))
        })
}

fn sync_option_card(
    title: SharedString,
    description: SharedString,
    colors: UiColors,
    body: Div,
) -> Div {
    div()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .p_4()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_baseline()
                .gap_2()
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(ui_text_size(10.5))
                        .text_color(colors.muted)
                        .child(description),
                ),
        )
        .child(body)
}

/// 选项卡片里的单行表单：标签定宽、控件定宽、说明占剩余宽度。
fn sync_form_row(
    label: SharedString,
    control: impl IntoElement,
    hint: SharedString,
    colors: UiColors,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(88.))
                .flex_none()
                .text_size(ui_text_size(11.))
                .child(label),
        )
        .child(control)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(ui_text_size(10.5))
                .text_color(colors.muted)
                .child(hint),
        )
}

fn sync_action_row(
    id: &'static str,
    label: SharedString,
    hint: SharedString,
    checked: bool,
    danger: bool,
    colors: UiColors,
    listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let mark = sync_action_mark(id);
    div()
        .id(id)
        .debug_selector(move || id.into())
        .px_2()
        .py_1p5()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(MouseButton::Left, listener)
        .child(Checkbox::new(id).checked(checked))
        .child(
            div()
                .text_size(ui_text_size(11.5))
                .text_color(if danger { colors.danger } else { colors.text })
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(ui_text_size(10.5))
                .text_color(colors.muted)
                .child(hint),
        )
        .when_some(mark, |this, mark| {
            this.child(
                div()
                    .flex_none()
                    .px_1p5()
                    .py_0p5()
                    .rounded(colors.radius)
                    .bg(colors.panel_alt)
                    .text_size(ui_text_size(9.5))
                    .text_color(if danger { colors.danger } else { colors.muted })
                    .child(mark),
            )
        })
}

/// 动作行右侧的英文标记，仅用于对齐设计稿的信息层级。
fn sync_action_mark(id: &'static str) -> Option<&'static str> {
    Some(match id {
        "sync-action-insert" => "INSERT",
        "sync-action-update" => "UPDATE",
        "sync-action-delete" => "DELETE",
        "sync-action-overwrite" => "OVERWRITE",
        "sync-action-create-missing" => "CREATE",
        "sync-structure-create-table" => "CREATE",
        "sync-structure-add-column" => "ADD",
        "sync-structure-alter-column" => "ALTER",
        "sync-structure-add-index" => "INDEX",
        "sync-structure-add-fk" => "FK",
        "sync-structure-drop-column" => "DROP",
        "sync-structure-drop-index" => "DROP",
        "sync-structure-drop-fk" => "DROP",
        "sync-structure-drop-table" => "DROP",
        _ => return None,
    })
}

/// 比对中的转圈图标。动画 id 单独一份，避免与侧边栏等其它加载动画互相重启。
fn sync_loading_spinner(size: f32, color: gpui::Rgba) -> impl IntoElement {
    svg()
        .size(px(size))
        .path("spinner.svg")
        .text_color(color)
        .with_animation(
            "sync_preview_loading_spinner",
            Animation::new(Duration::from_secs_f64(0.75))
                .repeat()
                .with_easing(ease_in_out),
            |this, delta| this.with_transformation(Transformation::rotate(percentage(delta))),
        )
}

/// 执行完成后的状态条：成功 / 部分失败 / 已取消都在这里给出结论，
/// 不再让界面停留在「刚打开预览」的样子。
fn sync_completion_banner(state: &SyncWizardState, colors: UiColors) -> Option<Div> {
    let completion = state.completion.as_ref()?;
    let (text, color) = match completion.status {
        SyncCompletionStatus::Success => match state.mode {
            SyncMode::Data => (
                tr("app.sync.completion_data")
                    .replace("%{tables}", &completion.tables.to_string())
                    .replace("%{inserted}", &completion.inserted.to_string())
                    .replace("%{updated}", &completion.updated.to_string())
                    .replace("%{deleted}", &completion.deleted.to_string()),
                colors.success,
            ),
            SyncMode::Structure => (
                tr("app.sync.completion_structure")
                    .replace("%{tables}", &completion.tables.to_string())
                    .replace("%{applied}", &completion.applied.to_string()),
                colors.success,
            ),
        },
        SyncCompletionStatus::Partial => match state.mode {
            SyncMode::Data => (
                tr("app.sync.completion_data_partial")
                    .replace("%{tables}", &completion.tables.to_string())
                    .replace("%{failed}", &completion.failed_tables.to_string())
                    .replace("%{inserted}", &completion.inserted.to_string())
                    .replace("%{updated}", &completion.updated.to_string())
                    .replace("%{deleted}", &completion.deleted.to_string()),
                colors.danger,
            ),
            SyncMode::Structure => (
                tr("app.sync.completion_structure_partial")
                    .replace("%{tables}", &completion.tables.to_string())
                    .replace("%{failed}", &completion.failed_tables.to_string())
                    .replace("%{applied}", &completion.applied.to_string()),
                colors.danger,
            ),
        },
        SyncCompletionStatus::Canceled => (
            tr("app.sync.completion_canceled").to_string(),
            colors.muted,
        ),
    };
    Some(
        div()
            .debug_selector(|| "sync-completion".into())
            .flex_none()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(colors.border_soft)
            .bg(colors.panel_alt)
            .text_size(ui_text_size(11.))
            .text_color(color)
            .child(text),
    )
}

fn sync_step_preview(
    state: &SyncWizardState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let body = match state.mode {
        SyncMode::Data => sync_data_preview(state, colors, cx),
        SyncMode::Structure => sync_structure_preview(state, colors, cx),
    };
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .when_some(sync_completion_banner(state, colors), |this, banner| {
            this.child(banner)
        })
        .when_some(state.preview_error.clone(), |this, error| {
            this.child(
                div()
                    .p_3()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.danger)
                    .child(error),
            )
        })
        .when(state.preview_running, |this| {
            // 比对期间占满预览区居中显示，避免转圈缩在左上角。
            this.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(sync_loading_spinner(14., colors.muted))
                    .child(tr("app.sync.preview_running")),
            )
        })
        .when(!state.preview_running, |this| this.child(body))
        .when(!state.log.is_empty(), |this| {
            this.child(sync_log_panel(state, colors))
        })
}

fn sync_data_preview(
    state: &SyncWizardState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(preview) = state.data_preview.as_ref() else {
        return div().flex_1();
    };
    let summary = div()
        .debug_selector(|| "sync-preview-summary".into())
        .flex_none()
        .h(px(42.))
        .px_4()
        .flex()
        .items_center()
        .gap_4()
        .border_b_1()
        .border_color(colors.border_soft)
        .text_size(ui_text_size(11.))
        .child(div().flex_1())
        .child(
            div()
                .text_color(colors.muted)
                .child(tr("app.sync.preview_note")),
        );
    let rows = preview
        .tables
        .iter()
        .map(|table| {
            (
                table,
                state.selected_table.as_deref() == Some(table.key.as_str()),
            )
        })
        .collect::<Vec<_>>();
    let detail = state
        .selected_table
        .as_deref()
        .and_then(|key| preview.tables.iter().find(|table| table.key == key))
        .map(|table| {
            sync_data_table_detail(
                table,
                state.data_sample_filter,
                state.page_running,
                colors,
                cx,
            )
        })
        .unwrap_or_else(|| {
            div()
                .p_4()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.sync.pick_table"))
        });
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .child(summary)
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .child(
                    div()
                        .debug_selector(|| "sync-preview-list".into())
                        .flex_none()
                        .w(px(280.))
                        .flex()
                        .flex_col()
                        .bg(colors.panel_alt)
                        .border_r_1()
                        .border_color(colors.border_soft)
                        .child(sync_preview_list_header(
                            state,
                            preview.tables.len(),
                            colors,
                            cx,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_h(px(0.))
                                .p_1()
                                .flex()
                                .flex_col()
                                .gap_0p5()
                                .overflow_y_scrollbar()
                                .children(rows.into_iter().map(|(table, selected)| {
                                    sync_preview_table_row(table, selected, colors, cx)
                                })),
                        ),
                )
                .child(div().flex_1().min_w(px(0.)).overflow_y_scrollbar().child(detail)),
        )
        .child(sync_preview_footer(
            state,
            tr("app.sync.change_stats")
                .replace("%{shown}", &shown_rows(preview).to_string())
                .replace("%{count}", &preview.total_changed().to_string()),
            colors,
            cx,
        ))
}

/// 预览左栏表头：对象计数 + 重新比较。重新比较复用后台比对，不写目标库。
fn sync_preview_list_header(
    state: &SyncWizardState,
    preview_tables: usize,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_none()
        .h(px(36.))
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .border_b_1()
        .border_color(colors.border_soft)
        .child(
            div()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_size(ui_text_size(11.))
                .child(tr("app.sync.sync_objects_label")),
        )
        .child(
            div()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .child(format!("{preview_tables}/{}", state.tables.len())),
        )
        .child(div().flex_1())
        .child(
            Button::new("sync-preview-recompare")
                .label(tr("app.sync.recompare"))
                .small()
                .disabled(state.preview_running || state.page_running || state.running)
                .on_click(cx.listener(|this, _, _, cx| this.start_sync_preview(cx))),
        )
}

/// 预览底部统计条，与设计稿一样只说明可见样本和总数。
fn sync_preview_footer(
    state: &SyncWizardState,
    text: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .debug_selector(|| "sync-preview-footer".into())
        .flex_none()
        .min_h(px(32.))
        .px_4()
        .flex()
        .items_center()
        .gap_2()
        .border_t_1()
        .border_color(colors.border_soft)
        .text_size(ui_text_size(10.5))
        .text_color(colors.muted)
        .child(text)
        .child(div().flex_1())
        // 执行完成后可回看落盘的块明细与行级样本；结构同步没有数据账本。
        .when(
            state.mode == SyncMode::Data && state.last_run_id.is_some(),
            |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .debug_selector(|| "sync-resume-run".into())
                                .child(
                                    Button::new("sync-resume-run")
                                        .label(tr("app.sync.resume_run"))
                                        .small()
                                        .disabled(state.running)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(run_id) = this
                                                .sync_wizard
                                                .as_ref()
                                                .and_then(|wizard| wizard.last_run_id)
                                            {
                                                this.start_sync_execute_with_resume(Some(run_id), cx);
                                            }
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .debug_selector(|| "sync-view-run-record".into())
                                .child(
                                    Button::new("sync-view-run-record")
                                        .label(tr("app.sync.view_record"))
                                        .small()
                                        .disabled(state.running)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(run_id) = this
                                                .sync_wizard
                                                .as_ref()
                                                .and_then(|wizard| wizard.last_run_id)
                                            {
                                                this.open_sync_run_view(Some(run_id), cx);
                                            }
                                        })),
                                ),
                        ),
                )
            },
        )
}

fn shown_rows(preview: &DataSyncPreview) -> usize {
    preview
        .tables
        .iter()
        .map(|table| table.samples.len())
        .sum()
}

fn sync_structure_preview(
    state: &SyncWizardState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(preview) = state.structure_preview.as_ref() else {
        return div().flex_1();
    };
    // 按表状态统计：新增/修改/删除/一致，与左侧徽标、右侧差异配色统一。
    let mut counts = [0u64; 4];
    for table in &preview.tables {
        match structure_table_kind(table) {
            StructureTableKind::Create => counts[0] += 1,
            StructureTableKind::Alter => counts[1] += 1,
            StructureTableKind::Drop => counts[2] += 1,
            StructureTableKind::Equal => counts[3] += 1,
            StructureTableKind::Skipped => {}
        }
    }
    let summary = div()
        .debug_selector(|| "sync-preview-summary".into())
        .flex_none()
        .h(px(42.))
        .px_4()
        .flex()
        .items_center()
        .gap_4()
        .border_b_1()
        .border_color(colors.border_soft)
        .text_size(ui_text_size(11.))
        .child(sync_sum_badge(tr("app.sync.change_create"), counts[0], colors))
        .child(sync_sum_badge(tr("app.sync.change_alter"), counts[1], colors))
        .child(sync_sum_badge(tr("app.sync.change_drop"), counts[2], colors))
        .child(sync_sum_badge(tr("app.sync.same"), counts[3], colors))
        .child(div().flex_1())
        // 差异布局：统一（上下对照）/ 并排（左右对照）。
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(structure_diff_layout_tab(
                    false,
                    tr("app.sync.diff_unified"),
                    !state.structure_diff_split,
                    colors,
                    cx,
                ))
                .child(structure_diff_layout_tab(
                    true,
                    tr("app.sync.diff_split"),
                    state.structure_diff_split,
                    colors,
                    cx,
                )),
        )
        .child(
            div()
                .text_color(colors.muted)
                .child(tr("app.sync.preview_note")),
        );
    let rows = preview
        .tables
        .iter()
        .map(|table| {
            (
                table,
                state.selected_table.as_deref() == Some(table.key.as_str()),
            )
        })
        .collect::<Vec<_>>();
    let detail = state
        .selected_table
        .as_deref()
        .and_then(|key| preview.tables.iter().find(|table| table.key == key))
        .map(|table| sync_structure_table_detail(table, state.structure_diff_split, colors))
        .unwrap_or_else(|| {
            div()
                .p_4()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.sync.pick_table"))
        });
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .child(summary)
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .child(
                    div()
                        .debug_selector(|| "sync-preview-list".into())
                        .flex_none()
                        .w(px(280.))
                        .flex()
                        .flex_col()
                        .bg(colors.panel_alt)
                        .border_r_1()
                        .border_color(colors.border_soft)
                        .child(sync_preview_list_header(
                            state,
                            preview.tables.len(),
                            colors,
                            cx,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_h(px(0.))
                                .p_1()
                                .flex()
                                .flex_col()
                                .gap_0p5()
                                .overflow_y_scrollbar()
                                .children(rows.into_iter().map(|(table, selected)| {
                                    sync_structure_table_row(table, selected, colors, cx)
                                })),
                        ),
                )
                .child(div().flex_1().min_w(px(0.)).overflow_y_scrollbar().child(detail)),
        )
        .child(sync_preview_footer(
            state,
            tr("app.sync.structure_change_stats")
                .replace("%{count}", &preview.total_changes().to_string()),
            colors,
            cx,
        ))
}

fn sync_preview_table_row(
    table: &TableDataDiff,
    selected: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let key = table.key.clone();
    div()
        .id(("sync-preview-row", sync_wizard_hash(&table.key)))
        .h(px(30.))
        .px_2()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .when(selected, |this| this.bg(colors.tree_selected))
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
            this.select_sync_preview_table(key.clone(), cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_size(ui_text_size(11.))
                .when(selected, |this| this.font_weight(gpui::FontWeight::SEMIBOLD))
                .child(table.name.clone()),
        )
        .child(sync_table_badge(table, colors))
        // 大表只比首块：只报「已对比多少行」，不给会被误读成整表结论的差异数。
        .when(table.skip.is_none() && table.error.is_none(), |this| {
            this.child(
                div()
                    .text_size(ui_text_size(10.))
                    .text_color(colors.muted)
                    .child(
                        tr("app.sync.compared_rows")
                            .replace("%{count}", &table.compared_rows().to_string()),
                    ),
            )
        })
}

/// 表级状态：只展示跳过原因或错误。
///
/// 差异计数不在这里展示：大表预览只比首块，片段数字挂到整表上会被误读；
/// 真实统计在执行完成后的记录页给出。
fn sync_table_badge(table: &TableDataDiff, colors: UiColors) -> Div {
    if let Some(skip) = table.skip {
        return div()
            .text_size(ui_text_size(10.))
            .text_color(colors.muted)
            .child(skip.message());
    }
    if let Some(error) = &table.error {
        return div()
            .text_size(ui_text_size(10.))
            .text_color(colors.muted)
            .child(error.clone());
    }
    div()
}

fn sync_structure_table_row(
    table: &TableStructureDiff,
    selected: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let key = table.key.clone();
    let badge = if let Some(skip) = table.skip {
        div()
            .text_size(ui_text_size(10.))
            .text_color(colors.muted)
            .child(skip.message())
    } else if let Some(error) = &table.error {
        div()
            .text_size(ui_text_size(10.))
            .text_color(colors.muted)
            .child(error.clone())
    } else {
        structure_state_chip(structure_table_kind(table), colors)
    };
    div()
        .id(("sync-structure-row", sync_wizard_hash(&table.key)))
        .h(px(30.))
        .px_2()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .when(selected, |this| this.bg(colors.tree_selected))
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
            this.select_sync_preview_table(key.clone(), cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_size(ui_text_size(11.))
                .when(selected, |this| this.font_weight(gpui::FontWeight::SEMIBOLD))
                .child(table.name.clone()),
        )
        .child(badge)
}

/// 列差异小标签：只显示数量，悬停查看完整说明与列名。
fn sync_column_diff_chip(
    id: String,
    label: SharedString,
    hint: String,
    colors: UiColors,
) -> Stateful<Div> {
    div()
        .id(gpui::ElementId::Name(id.into()))
        .px_1p5()
        .py_0p5()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.hover)
        .text_size(ui_text_size(10.))
        .text_color(colors.muted)
        .child(label)
        .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
}

fn sync_data_table_detail(
    table: &TableDataDiff,
    filter: Option<RowDiffAction>,
    page_running: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let hint = if !table.key_columns.is_empty() {
        let mut meta = format!(
            "{} {}",
            tr("app.sync.key_columns"),
            table.key_columns.join(", ")
        );
        if table.column_count > 0 {
            meta.push_str(" · ");
            meta.push_str(
                &tr("app.sync.compare_all_columns")
                    .replace("%{count}", &table.column_count.to_string()),
            );
        }
        Some(meta)
    } else {
        None
    };
    let samples = table
        .samples
        .iter()
        .filter(|sample| filter.is_none_or(|action| sample.action == action))
        .collect::<Vec<_>>();
    div()
        .p_4()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .font_family(crate::PLATFORM_MONOSPACE_FONT)
                        .text_size(ui_text_size(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(table.name.clone()),
                )
                .when_some(hint, |this, hint| {
                    this.child(
                        div()
                            .text_size(ui_text_size(10.5))
                            .text_color(colors.muted)
                            .child(hint),
                    )
                })
                // 列差异只显示数量，完整列名放悬停提示，避免详情区被说明文字占满。
                .when(!table.columns_only_in_source.is_empty(), |this| {
                    this.child(sync_column_diff_chip(
                        format!("sync-columns-src:{}", table.key),
                        tr("app.sync.columns_source_only_short")
                            .replace("%{count}", &table.columns_only_in_source.len().to_string())
                            .into(),
                        format!(
                            "{}: {}",
                            tr("app.sync.columns_only_in_source"),
                            table.columns_only_in_source.join(", ")
                        ),
                        colors,
                    ))
                })
                .when(!table.columns_only_in_target.is_empty(), |this| {
                    this.child(sync_column_diff_chip(
                        format!("sync-columns-dst:{}", table.key),
                        tr("app.sync.columns_target_only_short")
                            .replace("%{count}", &table.columns_only_in_target.len().to_string())
                            .into(),
                        format!(
                            "{}: {}",
                            tr("app.sync.columns_only_in_target"),
                            table.columns_only_in_target.join(", ")
                        ),
                        colors,
                    ))
                })
                .child(div().flex_1())
                // 样本筛选：按差异类型过滤可见样本。差异计数不展示（预览可能只是片段）。
                .child(sync_sample_filter_chip(
                    format!("sync-sample-filter:{}:all", table.key),
                    tr("app.sync.all_diffs"),
                    filter.is_none(),
                    None,
                    colors,
                    cx,
                ))
                .child(sync_sample_filter_chip(
                    format!("sync-sample-filter:{}:ins", table.key),
                    tr("app.sync.inserted"),
                    filter == Some(RowDiffAction::Insert),
                    Some(RowDiffAction::Insert),
                    colors,
                    cx,
                ))
                .child(sync_sample_filter_chip(
                    format!("sync-sample-filter:{}:upd", table.key),
                    tr("app.sync.updated"),
                    filter == Some(RowDiffAction::Update),
                    Some(RowDiffAction::Update),
                    colors,
                    cx,
                ))
                .child(sync_sample_filter_chip(
                    format!("sync-sample-filter:{}:del", table.key),
                    tr("app.sync.deleted"),
                    filter == Some(RowDiffAction::Delete),
                    Some(RowDiffAction::Delete),
                    colors,
                    cx,
                )),
        )
        .when_some(table.error.clone(), |this, error| {
            this.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.danger)
                    .child(error),
            )
        })
        .when_some(table.skip, |this, skip| {
            this.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(skip.message()),
            )
        })
        .when(samples.is_empty(), |this| {
            // 分页预览只比对了当前块，「本块无差异」不能说成整表一致。
            let text = if !table.page_done {
                tr("app.sync.page_no_diffs")
            } else if table.state == TableDiffState::Equal {
                tr("app.sync.tables_equal")
            } else {
                tr("app.sync.no_samples")
            };
            this.child(
                div()
                    .py_6()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(text),
            )
        })
        .when(!samples.is_empty(), |this| {
            this.child(
                // 样本表：主键 / 差异 / 字段 / 源值 / 目标值，逐行对照新旧值。
                div()
                    .rounded(colors.radius)
                    .border_1()
                    .border_color(colors.border_soft)
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(28.))
                            .flex()
                            .items_center()
                            .bg(colors.panel_alt)
                            .border_b_1()
                            .border_color(colors.border_soft)
                            .text_size(ui_text_size(10.))
                            .text_color(colors.muted)
                            .child(sync_sample_head_cell(
                                tr("app.sync.sample_key"),
                                px(SAMPLE_KEY_WIDTH),
                                colors,
                            ))
                            .child(sync_sample_head_cell(
                                tr("app.sync.sample_diff"),
                                px(SAMPLE_ACTION_WIDTH),
                                colors,
                            ))
                            .child(sync_sample_head_cell(
                                tr("app.sync.sample_column"),
                                px(SAMPLE_COLUMN_WIDTH),
                                colors,
                            ))
                            .child(sync_sample_head_cell(
                                tr("app.sync.sample_source"),
                                px(0.),
                                colors,
                            ))
                            .child(sync_sample_head_cell(
                                tr("app.sync.sample_target"),
                                px(0.),
                                colors,
                            )),
                    )
                    .children(samples.into_iter().map(|sample| {
                        sync_sample_row(sample, table.column_count, colors)
                    })),
            )
        })
        .when(!table.page_done, |this| {
            this.child(sync_data_pager(table, page_running, colors, cx))
        })
}

/// 分页器：大表预览只比首块，点一次「下一页」再比一块。
fn sync_data_pager(
    table: &TableDataDiff,
    page_running: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .debug_selector(|| "sync-data-pager".into())
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .text_size(ui_text_size(10.5))
        .text_color(colors.muted)
        .child(tr("app.sync.preview_partial"))
        .child(div().flex_1())
        .child(
            // gpui-component 的 Button 不暴露 debug 选择器，外包一层供端到端测试定位。
            div()
                .debug_selector({
                    let key = table.key.clone();
                    move || format!("sync-next-page:{}", key)
                })
                .child(
                    Button::new(format!("sync-next-page:{}", table.key))
                        .label(tr("app.sync.next_page"))
                        .small()
                        .disabled(page_running)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.start_sync_next_page(cx);
                        })),
                ),
        )
}

const SAMPLE_KEY_WIDTH: f32 = 150.;
const SAMPLE_ACTION_WIDTH: f32 = 76.;
const SAMPLE_COLUMN_WIDTH: f32 = 140.;

/// 样本表头单元格；`width` 为 0 时均分剩余宽度（源值/目标值）。
fn sync_sample_head_cell(label: SharedString, width: Pixels, colors: UiColors) -> Div {
    let cell = div()
        .px_2()
        .flex_none()
        .text_size(ui_text_size(10.))
        .text_color(colors.muted);
    if width == px(0.) {
        cell.flex_1().min_w(px(0.)).child(label)
    } else {
        cell.w(width).child(label)
    }
}

/// 样本行：左侧色条 + 主键 / 差异 / 字段 / 源值 / 目标值。
fn sync_sample_row(sample: &RowDiffSample, column_count: usize, colors: UiColors) -> Div {
    let (chip_label, color) = match sample.action {
        RowDiffAction::Insert => (tr("app.sync.inserted"), colors.success),
        RowDiffAction::Update => (tr("app.sync.updated"), colors.warning),
        RowDiffAction::Delete => (tr("app.sync.deleted"), colors.danger),
    };
    let key = sample
        .key_values
        .iter()
        .map(|(name, value)| format!("{name} = {value}"))
        .collect::<Vec<_>>()
        .join(", ");
    let all_columns = tr("app.sync.sample_all_columns")
        .replace("%{count}", &column_count.to_string())
        .into();
    let absent = tr("app.sync.sample_absent");
    // 新增/删除按整行处理：值侧分别显示「不存在」与「全部 N 列」。
    let (source, target) = match sample.action {
        RowDiffAction::Insert => (
            SampleValue::plain(all_columns),
            SampleValue::plain(absent),
        ),
        RowDiffAction::Delete => (
            SampleValue::plain(absent),
            SampleValue::plain(all_columns),
        ),
        RowDiffAction::Update => (
            SampleValue::tinted(
                sample
                    .source_value
                    .clone()
                    .unwrap_or_else(|| absent.to_string()),
                colors.danger,
            ),
            SampleValue::tinted(
                sample
                    .target_value
                    .clone()
                    .unwrap_or_else(|| absent.to_string()),
                colors.success,
            ),
        ),
    };
    div()
        .h(px(30.))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(colors.border_soft)
        .child(div().w(px(2.)).flex_none().self_stretch().bg(color))
        .child(
            div()
                .w(px(SAMPLE_KEY_WIDTH - 2.))
                .flex_none()
                .px_2()
                .truncate()
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_size(ui_text_size(10.5))
                .child(key),
        )
        .child(
            div().w(px(SAMPLE_ACTION_WIDTH)).flex_none().px_2().child(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded(colors.radius)
                    .bg(color.alpha(0.12))
                    .text_size(ui_text_size(10.))
                    .text_color(color)
                    .child(chip_label),
            ),
        )
        .child(match sample.column.clone() {
            Some(column) => div()
                .w(px(SAMPLE_COLUMN_WIDTH))
                .flex_none()
                .px_2()
                .truncate()
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_size(ui_text_size(10.5))
                .child(column),
            None => div()
                .w(px(SAMPLE_COLUMN_WIDTH))
                .flex_none()
                .px_2()
                .text_size(ui_text_size(10.5))
                .text_color(colors.muted)
                .child("—"),
        })
        .child(sync_sample_value_cell(source, colors))
        .child(sync_sample_value_cell(target, colors))
}

/// 样本值单元格的内容：普通文本，或带说明性底色的新旧值。
struct SampleValue {
    text: SharedString,
    tint: Option<gpui::Rgba>,
}

impl SampleValue {
    fn plain(text: SharedString) -> Self {
        Self { text, tint: None }
    }

    fn tinted(text: String, tint: gpui::Rgba) -> Self {
        Self {
            text: text.into(),
            tint: Some(tint),
        }
    }
}

fn sync_sample_value_cell(value: SampleValue, colors: UiColors) -> Div {
    let cell = div()
        .flex_1()
        .min_w(px(0.))
        .px_2()
        .flex()
        .items_center()
        .font_family(crate::PLATFORM_MONOSPACE_FONT)
        .text_size(ui_text_size(10.5));
    match value.tint {
        Some(tint) => cell.child(
            div()
                .px_1()
                .py_0p5()
                .rounded(colors.radius)
                .bg(tint.alpha(0.12))
                .truncate()
                .child(value.text),
        ),
        None => cell.text_color(colors.muted).child(value.text),
    }
}

/// 样本筛选标签：点击切换，「全部」表示不筛选。
fn sync_sample_filter_chip(
    id: String,
    label: SharedString,
    active: bool,
    filter: Option<RowDiffAction>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    div()
        .id(gpui::ElementId::Name(id.into()))
        .px_1p5()
        .py_0p5()
        .rounded(colors.radius)
        .border_1()
        .border_color(if active {
            colors.border
        } else {
            colors.border_soft
        })
        .when(active, |this| this.bg(colors.tree_selected))
        .text_size(ui_text_size(10.))
        .text_color(if active { colors.text } else { colors.muted })
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
            this.set_sync_sample_filter(filter, cx);
            cx.stop_propagation();
        }))
        .child(label)
}

fn sync_structure_table_detail(
    table: &TableStructureDiff,
    split: bool,
    colors: UiColors,
) -> Div {
    let kind = structure_table_kind(table);
    let detail = div()
        .p_4()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .font_family(crate::PLATFORM_MONOSPACE_FONT)
                        .text_size(ui_text_size(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(table.name.clone()),
                )
                .child(structure_state_chip(kind, colors)),
        )
        .when_some(table.error.clone(), |this, error| {
            this.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.danger)
                    .child(error),
            )
        })
        .when_some(table.skip, |this, skip| {
            this.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(skip.message()),
            )
        })
        .when(!table.warnings.is_empty(), |this| {
            this.child(
                div()
                    .debug_selector(|| "sync-structure-warnings".into())
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.warning)
                            .child(tr("app.sync.warnings")),
                    )
                    .children(table.warnings.iter().map(|warning| {
                        div()
                            .text_size(ui_text_size(10.5))
                            .text_color(colors.warning)
                            .child(warning.clone())
                    })),
            )
        });
    // 两端整表定义齐全时用 git 风格逐行差异；缺少任一侧（如待删除的
    // 目标表没有读取元数据）时退回按变更列条目展示。
    match table.desired_ddl.as_deref() {
        Some(desired) => detail.child(structure_ddl_diff_view(
            table.current_ddl.as_deref().unwrap_or_default(),
            desired,
            split,
            colors,
        )),
        None => detail.child(sync_structure_change_list(table, colors)),
    }
}

/// 兜底视图：按对象逐条列出变更，仅在拿不到两端整表定义时使用。
fn sync_structure_change_list(table: &TableStructureDiff, colors: UiColors) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .children(table.changes.iter().map(|change| {
            let action = match change.action {
                StructureChangeAction::Create => tr("app.sync.change_create"),
                StructureChangeAction::Alter => tr("app.sync.change_alter"),
                StructureChangeAction::Drop => tr("app.sync.change_drop"),
            };
            let object = match change.object {
                StructureObject::Table => tr("app.sync.object_table"),
                StructureObject::Column => tr("app.sync.object_column"),
                StructureObject::PrimaryKey => tr("app.sync.object_primary_key"),
                StructureObject::Index => tr("app.sync.object_index"),
                StructureObject::ForeignKey => tr("app.sync.object_foreign_key"),
                StructureObject::Check => tr("app.sync.object_check"),
            };
            let before = change.before_text().unwrap_or_else(|| "—".to_string());
            let after = change.after_text().unwrap_or_else(|| "—".to_string());
            div()
                .px_2()
                .py_1()
                .rounded(colors.radius)
                .bg(colors.hover)
                .text_size(ui_text_size(10.5))
                .child(format!(
                    "{action} {object} {} · {before} → {after}",
                    change.name
                ))
        }))
}

/// 消息日志面板：标题 + 可滚动正文，执行期间实时追加。
///
/// 只保留在预览步骤底部（执行中才出现），不遮预览内容；正文区自己带滚动条，
/// 追加新行时贴底跟随。
fn sync_log_panel(state: &SyncWizardState, colors: UiColors) -> impl IntoElement {
    let rows = state.log.iter().map(|entry| {
        div()
            .flex()
            .items_start()
            .gap_2()
            .child(
                div()
                    .flex_none()
                    .w(px(58.))
                    .text_color(colors.muted)
                    .child(entry.at.clone()),
            )
            .child(div().flex_1().min_w(px(0.)).child(entry.text.clone()))
    });
    div()
        .debug_selector(|| "sync-log-panel".into())
        .flex_none()
        .h(px(160.))
        .border_t_1()
        .border_color(colors.border_soft)
        .flex()
        .flex_col()
        .child(
            div()
                .flex_none()
                .h(px(28.))
                .px_3()
                .flex()
                .items_center()
                .border_b_1()
                .border_color(colors.border_soft)
                .text_size(ui_text_size(10.5))
                .text_color(colors.muted)
                .child(tr("app.sync.log_title")),
        )
        .child(
            // 滚动能力（track_scroll / overflow_y_scroll）只在 Stateful 元素上，
            // 滚动条挂在同级 relative 容器上（与字段设置弹框一致）。
            div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .w_full()
                .child(
                    div()
                        .id("sync-log-body")
                        .size_full()
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .track_scroll(&state.log_scroll)
                        .overflow_y_scroll()
                        .text_size(ui_text_size(10.5))
                        .text_color(colors.text)
                        .children(rows),
                )
                .vertical_scrollbar(&state.log_scroll),
        )
}

fn sync_step_hint(step: u8) -> SharedString {
    match step {
        1 => tr("app.sync.step1_hint"),
        2 => tr("app.sync.step2_hint"),
        3 => tr("app.sync.step3_hint"),
        _ => tr("app.sync.step4_hint"),
    }
}

fn sync_footer(
    state: &SyncWizardState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let busy = state.running || state.preview_running;
    // 执行结束后主按钮改为「完成」：关闭向导，结果已固定在状态条与任务中心。
    let finished = state.completion.is_some() && !state.running;
    div()
        .debug_selector(|| "sync-wizard-footer".into())
        .flex_none()
        .h(px(56.))
        .px_5()
        .flex()
        .items_center()
        .gap_2()
        .border_t_1()
        .border_color(colors.border_soft)
        .child(
            div()
                .debug_selector(|| "sync-footer-hint".into())
                .text_size(ui_text_size(10.5))
                .text_color(colors.muted)
                .child(sync_step_hint(state.step)),
        )
        .when_some(state.error.clone(), |this, error| {
            this.child(
                div()
                    .text_size(ui_text_size(10.5))
                    .text_color(colors.danger)
                    .child(error),
            )
        })
        .child(div().flex_1())
        .when(busy, |this| {
            this.child(
                div()
                    .text_size(ui_text_size(10.5))
                    .text_color(colors.muted)
                    .child(tr("app.sync.background_hint")),
            )
        })
        .child(
            div()
                .debug_selector(|| "sync-wizard-cancel".into())
                .child(
            Button::new("sync-wizard-cancel")
                .label(if state.running {
                    tr("app.sync.stop")
                } else {
                    tr("app.sync.cancel")
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(state) = this.sync_wizard.as_ref()
                        && state.running
                    {
                        state.cancel.store(true, Ordering::Relaxed);
                    } else {
                        this.close_sync_wizard(cx);
                    }
                }))),
        )
        .when(state.step > 1, |this| {
            this.child(
                div()
                    .debug_selector(|| "sync-wizard-back".into())
                    .child(
                Button::new("sync-wizard-back")
                    .label(tr("app.sync.back"))
                    .disabled(state.running)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let step = this
                            .sync_wizard
                            .as_ref()
                            .map(|state| state.step.saturating_sub(1))
                            .unwrap_or(1);
                        this.go_sync_step(step, window, cx);
                    }))),
            )
        })
        .when(state.step < 4, |this| {
            this.child(
                div()
                    .debug_selector(|| "sync-wizard-next".into())
                    .child(
                Button::new("sync-wizard-next")
                    .label(if state.preview_running {
                        tr("app.sync.preview_running")
                    } else if state.step == 3 {
                        tr("app.sync.preview_next")
                    } else {
                        tr("app.sync.next")
                    })
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let step = this
                            .sync_wizard
                            .as_ref()
                            .map(|state| state.step.saturating_add(1))
                            .unwrap_or(2);
                        this.go_sync_step(step, window, cx);
                    }))),
            )
        })
        .when(state.step == 4, |this| {
            this.child(
                div()
                    .debug_selector(|| "sync-wizard-run".into())
                    .child(
                Button::new("sync-wizard-run")
                    .label(if state.running {
                        tr("app.sync.running")
                    } else if finished {
                        tr("app.sync.done")
                    } else {
                        tr("app.sync.run")
                    })
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if finished {
                            this.close_sync_wizard(cx);
                        } else {
                            this.start_sync_execute(cx);
                        }
                    }))),
            )
        })
}

fn sync_sum_badge(label: SharedString, count: u64, colors: UiColors) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(div().text_color(colors.muted).child(label))
        .child(
            div()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(count.to_string()),
        )
}

fn sync_table_count_label(state: &SyncWizardState) -> String {
    tr("app.sync.table_count")
        .replace("%{total}", &state.tables.len().to_string())
        .replace("%{selected}", &state.selected_tables.len().to_string())
}

fn sync_connection_name(connections: &[ConnectionConfig], connection_id: ConnectionId) -> String {
    connections
        .iter()
        .find(|connection| connection.id == connection_id)
        .map(|connection| connection.name.clone())
        .unwrap_or_default()
}

fn sync_wizard_hash(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}
