fn data_filter_panel(
    tab_id: TabId,
    sql: String,
    sql_input: Entity<InputState>,
    page: &DataPage,
    rules: &[DataFilterRule],
    sort_rules: &[DataSortRule],
    clear_sort_undo_available: bool,
    mode: DataFilterMode,
    panel_height: f32,
    panel_max_height: f32,
    applying: bool,
    data_filter_text_input: Entity<gpui_component::input::EditorState>,
    _database_kind: DatabaseKind,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let field_names = page
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let default_field = field_names.first().cloned();
    let default_field_for_row = default_field.clone();
    let default_sort_field = default_field.clone();
    let view = cx.entity();
    div()
        .debug_selector(|| "data-filter-panel".into())
        .relative()
        .h(px(panel_height))
        .overflow_hidden()
        .border_b_1()
        .border_color(colors.border)
        .bg(if colors.is_dark {
            rgb(0x15181d)
        } else {
            rgb(0xffffff)
        })
        .flex()
        .flex_col()
        .child(
            div()
                .debug_selector(|| "data-filter-scroll-area".into())
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .child(match mode {
                    DataFilterMode::Builder => data_filter_builder_rows(
                        tab_id,
                        rules,
                        default_field_for_row,
                        colors,
                        cx,
                    ),
                    DataFilterMode::Text => data_filter_condition_editor(
                        data_filter_text_input,
                        colors,
                    ),
                }),
        )
        .child(
            div()
                .debug_selector(|| "data-filter-fixed-actions".into())
                .flex_shrink_0()
                .border_t_1()
                .border_color(colors.border_soft),
        )
        .child(data_sort_builder_section(
            tab_id,
            sort_rules,
            clear_sort_undo_available,
            default_sort_field,
            colors,
            cx,
        ))
        .child(
            div()
                .debug_selector(|| "data-filter-apply-row".into())
                .h(px(38.))
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .child(data_filter_apply_button(colors, applying).on_mouse_down(
                    MouseButton::Left,
                    move |_, _, cx| {
                        if applying {
                            cx.stop_propagation();
                            return;
                        }
                        view.update(cx, |this, cx| {
                            this.apply_data_filter_and_sort(tab_id, cx);
                        });
                        cx.stop_propagation();
                    },
                ))
                .child(data_editor_sql_input(sql, sql_input, colors, window, cx)),
        )
        .child(data_filter_panel_resize_handle(
            tab_id,
            panel_height,
            panel_max_height,
            colors,
            cx,
        ))
}

fn data_filter_apply_button(colors: UiColors, applying: bool) -> Div {
    let button = div()
        .debug_selector(|| "data-filter-apply".into())
        .h(px(26.))
        .px_3()
        .rounded(colors.radius)
        .border_1()
        .border_color(if colors.is_dark {
            rgb(0x4a90ff)
        } else {
            rgb(0x1d7ff2)
        })
        .bg(rgb(0x2f8df6))
        .text_size(ui_text_size(12.))
        .text_color(rgb(0xffffff))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .shadow(vec![box_shadow(
            0.,
            1.,
            2.,
            0.,
            hsla(211. / 360., 0.83, 0.42, 0.22),
        )])
        .hover(|style| style.bg(rgb(0x1677ff)).border_color(rgb(0x1677ff)));

    if applying {
        button.opacity(0.86).child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(loading_spinner_with_color(12., rgb(0xffffff)))
                .child(tr("app.filter.applying")),
        )
    } else {
        button.child(tr("app.filter.apply_filters_sort"))
    }
}

fn data_filter_toolbar_mode_switch(
    tab_id: TabId,
    mode: DataFilterMode,
    colors: UiColors,
    dialect: sql_editor_adapter::SqlDialect,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .debug_selector(|| "data-filter-toolbar-mode-switch".into())
        .h(px(28.))
        .flex()
        .items_center()
        .gap_1()
        .flex_shrink_0()
        .child(data_filter_mode_button(
            "data-filter-mode-builder",
            tr("app.filter.mode.builder"),
            mode == DataFilterMode::Builder,
            colors,
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.set_data_filter_mode(
                    tab_id,
                    DataFilterMode::Builder,
                    dialect,
                    window,
                    cx,
                );
                cx.stop_propagation();
            }),
        ))
        .child(data_filter_mode_button(
            "data-filter-mode-input",
            tr("app.filter.mode.input"),
            mode == DataFilterMode::Text,
            colors,
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.set_data_filter_mode(
                    tab_id,
                    DataFilterMode::Text,
                    dialect,
                    window,
                    cx,
                );
                cx.stop_propagation();
            }),
        ))
}

fn data_filter_mode_button(
    debug_id: &'static str,
    label: SharedString,
    active: bool,
    colors: UiColors,
) -> Div {
    div()
        .debug_selector(move || debug_id.into())
        .h(px(22.))
        .min_w(px(48.))
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(if active { rgb(0x1677ff) } else { colors.border })
        .bg(if active {
            if colors.is_dark {
                rgb(0x1a3157)
            } else {
                rgb(0xeaf3ff)
            }
        } else {
            colors.panel_bg
        })
        .text_size(ui_text_size(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if active { rgb(0x006bd6) } else { colors.muted })
        .cursor_pointer()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover))
        .child(label)
}

fn filter_combinator_label(combinator: fluxdb_core::FilterCombinator) -> &'static str {
    match combinator {
        fluxdb_core::FilterCombinator::And => "and",
        fluxdb_core::FilterCombinator::Or => "or",
    }
}

fn data_filter_condition_editor(
    input: Entity<gpui_component::input::EditorState>,
    colors: UiColors,
) -> Div {
    div()
        .debug_selector(|| "data-filter-condition-editor".into())
        .h(px(112.))
        .px_3()
        .py_2()
        .child(
            div()
                .size_full()
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .overflow_hidden()
                .child(
                    gpui_component::input::Editor::new(&input)
                        .appearance(false)
                        .bordered(false)
                        .h_full()
                        .text_size(ui_text_size(12.))
                        .aria_label(tr("app.filter.mode.input")),
                ),
        )
}

fn data_filter_panel_resize_handle(
    tab_id: TabId,
    panel_height: f32,
    panel_max_height: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id("data-filter-panel-resize-handle")
        .absolute()
        .left_0()
        .right_0()
        .bottom(px(-3.))
        .h(px(7.))
        .cursor_ns_resize()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .h(px(1.))
                .w_full()
                .bg(colors.border_soft)
                .hover(|style| style.bg(rgb(0x3478f6))),
        )
        .hover(|style| style.bg(hsla(212. / 360., 0.92, 0.58, 0.10)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.data_filter_panel_resize_start = Some(DataFilterPanelResizeStart {
                    tab_id,
                    y: f32::from(event.position.y),
                    height: panel_height,
                });
                this.data_filter_popover = None;
                this.sync_table_hover_overlay_block(cx);
                cx.stop_propagation();
            }),
        )
        .on_drag(DataFilterPanelResizeDrag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_drag_move(cx.listener(
            move |this, event: &DragMoveEvent<DataFilterPanelResizeDrag>, _, cx| {
                if let Some(start) = this.data_filter_panel_resize_start {
                    if start.tab_id == tab_id {
                        let delta = f32::from(event.event.position.y) - start.y;
                        this.data_filter_panel_heights
                            .insert(
                                tab_id,
                                (start.height + delta)
                                    .clamp(DATA_FILTER_PANEL_MIN_HEIGHT, panel_max_height),
                            );
                        cx.notify();
                    }
                }
                cx.stop_propagation();
            },
        ))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.data_filter_panel_resize_start = None;
                cx.stop_propagation();
            }),
        )
}

fn data_editor_sql_preview(
    object: &ObjectPath,
    columns: &[GdbColumn],
    rules: &[DataFilterRule],
    sort_rules: &[DataSortRule],
    mode: DataFilterMode,
    filter_text: &str,
    sort_text: &str,
    offset: u64,
    limit: u64,
    db_kind: DatabaseKind,
) -> String {
    let sqlserver = db_kind == DatabaseKind::SqlServer;
    let mut clauses = Vec::new();
    let mut has_order_by = false;
    match mode {
        DataFilterMode::Builder => {
            let where_clause = data_filter_rules_sql(rules, db_kind, columns);
            if !where_clause.is_empty() {
                clauses.push(format!("WHERE {where_clause}"));
            }
            let order_by = sort_rules
                .iter()
                .filter(|rule| rule.enabled)
                .map(|rule| {
                    let direction = if rule.ascending { "ASC" } else { "DESC" };
                    format!("{} {direction}", sql_quote_ident(&rule.field, db_kind))
                })
                .collect::<Vec<_>>();
            if !order_by.is_empty() {
                clauses.push(format!("ORDER BY {}", order_by.join(", ")));
                has_order_by = true;
            }
        }
        DataFilterMode::Text => {
            let filter_text = filter_text.trim();
            if !filter_text.is_empty() {
                clauses.push(format!("WHERE {filter_text}"));
            }
            let sort_text = sort_text.trim();
            if !sort_text.is_empty() {
                clauses.push(format!("ORDER BY {sort_text}"));
                has_order_by = true;
            }
        }
    }

    if sqlserver && !has_order_by {
        // T-SQL 的 OFFSET/FETCH 需要 ORDER BY；首页用 TOP，翻页补稳定占位排序。
        if offset == 0 {
            return format!(
                "SELECT TOP ({limit}) * FROM {} {}",
                sql_qualified_object_name(object, db_kind),
                clauses.join(" ")
            )
            .trim_end()
            .to_string();
        }
        clauses.push("ORDER BY (SELECT NULL)".to_string());
    }

    // 分页子句统一由方言渲染（其余关系型数据库为 `LIMIT … OFFSET …`）。
    let pagination = sql_dialect(db_kind).render_limit_offset(limit, offset);
    clauses.push(pagination.trim_start().to_string());

    format!(
        "SELECT * FROM {} {}",
        sql_qualified_object_name(object, db_kind),
        clauses.join(" ")
    )
}

