fn data_filter_panel(
    tab_id: TabId,
    sql: String,
    sql_input: Entity<InputState>,
    page: &DataPage,
    rules: &[DataFilterRule],
    sort_rules: &[DataSortRule],
    _mode: DataFilterMode,
    panel_height: f32,
    applying: bool,
    _data_filter_text_input: Entity<InputState>,
    _data_sort_text_input: Entity<InputState>,
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
                .h(px(28.))
                .px_3()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("筛选"),
                ),
        )
        .child(data_filter_builder_rows(
            tab_id,
            rules,
            default_field_for_row,
            colors,
            cx,
        ))
        .child(div().flex_1().border_b_1().border_color(colors.border_soft))
        .child(data_sort_builder_section(
            tab_id,
            sort_rules,
            default_sort_field,
            colors,
            cx,
        ))
        .child(
            div()
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
            colors,
            cx,
        ))
}

fn data_filter_apply_button(colors: UiColors, applying: bool) -> Div {
    let button = div()
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
        .text_size(px(12.))
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
                .child("正在应用..."),
        )
    } else {
        button.child("应用筛选 & 排序")
    }
}

fn data_filter_panel_resize_handle(
    tab_id: TabId,
    panel_height: f32,
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
                            .insert(tab_id, clamp_data_filter_panel_height(start.height + delta));
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
    rules: &[DataFilterRule],
    sort_rules: &[DataSortRule],
    mode: DataFilterMode,
    filter_text: &str,
    sort_text: &str,
    offset: u64,
    limit: u64,
) -> String {
    let mut clauses = Vec::new();
    match mode {
        DataFilterMode::Builder => {
            let where_clause = data_filter_rules_sql(rules);
            if !where_clause.is_empty() {
                clauses.push(format!("WHERE {where_clause}"));
            }
            let order_by = sort_rules
                .iter()
                .filter(|rule| rule.enabled)
                .map(|rule| {
                    let direction = if rule.ascending { "ASC" } else { "DESC" };
                    format!("{} {direction}", sql_quote_ident(&rule.field))
                })
                .collect::<Vec<_>>();
            if !order_by.is_empty() {
                clauses.push(format!("ORDER BY {}", order_by.join(", ")));
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
            }
        }
    }

    clauses.push(format!("LIMIT {limit}"));
    if offset > 0 {
        clauses.push(format!("OFFSET {offset}"));
    }

    format!(
        "SELECT * FROM {} {}",
        sql_qualified_object_name(object),
        clauses.join(" ")
    )
}

