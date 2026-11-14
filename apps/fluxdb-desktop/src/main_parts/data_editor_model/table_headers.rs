fn data_table_header_tooltip(meta: DataTableColumnMeta, type_color: gpui::Rgba) -> Tooltip {
    Tooltip::element(move |_, cx| {
        div()
            .w(px(220.))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(data_table_header_tooltip_row(
                tr("app.data.header.column"),
                localized_column_header(&meta.name).to_string(),
                cx.theme().popover_foreground,
            ))
            .child(data_table_header_tooltip_row(
                tr("app.data.header.type"),
                meta.type_name.clone(),
                type_color,
            ))
            .child(data_table_header_tooltip_row(
                tr("app.data.header.attributes"),
                data_table_column_attr_label(&meta),
                cx.theme().popover_foreground,
            ))
            .child(data_table_header_tooltip_row(
                tr("app.data.header.comment"),
                data_table_column_comment_label(&meta),
                cx.theme().popover_foreground,
            ))
    })
}

fn data_table_header_tooltip_row(
    label: impl Into<SharedString>,
    value: String,
    value_color: impl Into<Hsla>,
) -> impl IntoElement {
    let label: SharedString = label.into();
    let tooltip_value_selector = label.clone();
    div()
        .flex()
        .items_start()
        .gap_3()
        .text_size(ui_text_size(13.))
        .child(
            div()
                .w(px(34.))
                .flex_none()
                .text_color(rgb(0x7b8190))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_normal()
                .text_color(value_color)
                .debug_selector(move || {
                    format!("data-table-header-tooltip-value:{tooltip_value_selector}")
                })
                .child(value),
        )
}

fn data_table_column_comment_label(meta: &DataTableColumnMeta) -> String {
    meta.comment
        .as_deref()
        .map(str::trim)
        .filter(|comment| !comment.is_empty())
        .map(str::to_string).unwrap_or_else(|| tr("app.data.column.none").to_string())
        .to_string()
}

fn data_table_column_attr_label(meta: &DataTableColumnMeta) -> String {
    match (meta.primary_key, meta.nullable) {
        (true, true) => tr("app.data.column.pk_nullable").to_string(),
        (true, false) => tr("app.data.column.pk_not_null").to_string(),
        (false, true) => tr("app.data.column.nullable").to_string(),
        (false, false) => tr("app.data.column.not_null").to_string(),
    }
}

fn data_table_header_sort_popover(
    tab_id: TabId,
    pane_id: QueryPaneId,
    col_ix: usize,
    column_name: String,
    sort_icon: &'static str,
    active: Option<DataTableSortDirection>,
    on_sort: DataTableSortHandler,
    icon_color: Hsla,
) -> impl IntoElement {
    Popover::new(("data-table-sort-popover", col_ix))
        .appearance(false)
        .anchor(Anchor::TopRight)
        .trigger(data_table_header_text_button(
            ("data-table-sort", col_ix),
            sort_icon,
            tr("app.data.header.sort"),
            icon_color,
        ))
        .content(move |_, _, cx| {
            let popover = cx.entity();
            data_table_header_menu_shell(cx)
                .w(px(132.))
                .child(
                    data_table_header_menu_item(
                        "↑",
                        tr("app.data.sort.ascending"),
                        active == Some(DataTableSortDirection::Ascending),
                        cx,
                    )
                    .on_mouse_down(MouseButton::Left, {
                        let column_name = column_name.clone();
                        let on_sort = on_sort.clone();
                        let popover = popover.clone();
                        move |_, window, cx| {
                            on_sort(
                                QueryPaneKey::new(tab_id, pane_id),
                                column_name.clone(),
                                Some(DataTableSortDirection::Ascending),
                                cx,
                            );
                            popover.update(cx, |state, cx| {
                                state.dismiss(window, cx);
                            });
                            cx.stop_propagation();
                        }
                    }),
                )
                .child(
                    data_table_header_menu_item(
                        "↓",
                        tr("app.data.sort.descending"),
                        active == Some(DataTableSortDirection::Descending),
                        cx,
                    )
                    .on_mouse_down(MouseButton::Left, {
                        let column_name = column_name.clone();
                        let on_sort = on_sort.clone();
                        let popover = popover.clone();
                        move |_, window, cx| {
                            on_sort(
                                QueryPaneKey::new(tab_id, pane_id),
                                column_name.clone(),
                                Some(DataTableSortDirection::Descending),
                                cx,
                            );
                            popover.update(cx, |state, cx| {
                                state.dismiss(window, cx);
                            });
                            cx.stop_propagation();
                        }
                    }),
                )
                .child(
                    data_table_header_menu_item("⊘", tr("app.data.sort.remove"), false, cx).on_mouse_down(
                        MouseButton::Left,
                        {
                            let column_name = column_name.clone();
                            let on_sort = on_sort.clone();
                            let popover = popover.clone();
                            move |_, window, cx| {
                                on_sort(
                                    QueryPaneKey::new(tab_id, pane_id),
                                    column_name.clone(),
                                    None,
                                    cx,
                                );
                                popover.update(cx, |state, cx| {
                                    state.dismiss(window, cx);
                                });
                                cx.stop_propagation();
                            }
                        },
                    ),
                )
        })
}

fn data_table_header_action_popover(
    tab_id: TabId,
    col_ix: usize,
    column_name: String,
    view: WeakEntity<NavicatMain>,
    _table: Entity<TableState<DataPageTableDelegate>>,
    icon_color: Hsla,
) -> impl IntoElement {
    Popover::new(("data-table-action-popover", col_ix))
        .appearance(false)
        .anchor(Anchor::TopRight)
        .trigger(data_table_header_text_button(
            ("data-table-actions", col_ix),
            "▾",
            tr("app.data.header.column_actions"),
            icon_color,
        ))
        .content(move |_, _, cx| {
            let popover = cx.entity();
            data_table_header_menu_shell(cx)
                .w(px(160.))
                .child(
                    data_table_header_menu_item("⧉", tr("app.data.action.copy_column_name"), false, cx).on_mouse_down(
                        MouseButton::Left,
                        {
                            let column_name = column_name.clone();
                            let view = view.clone();
                            let popover = popover.clone();
                            move |_, window, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    column_name.clone(),
                                ));
                                let _ = view.update(cx, |this, cx| {
                                    this.show_message(tr("app.data.toast.copied").as_ref(), AppMessageKind::Success, cx);
                                });
                                popover.update(cx, |state, cx| {
                                    state.dismiss(window, cx);
                                });
                                cx.stop_propagation();
                            }
                        },
                    ),
                )
                .child(
                    data_table_header_menu_item_icon(AppIcon::List, tr("app.data.action.set_enum"), false, cx)
                        .on_mouse_down(MouseButton::Left, {
                            let column_name = column_name.clone();
                            let view = view.clone();
                            let popover = popover.clone();
                            move |_, window, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.show_column_choices_modal(
                                        tab_id,
                                        column_name.clone(),
                                        window,
                                        cx,
                                    );
                                });
                                popover.update(cx, |state, cx| {
                                    state.dismiss(window, cx);
                                });
                                cx.stop_propagation();
                            }
                        }),
                )
                .child(data_table_header_menu_item("⇩", tr("app.data.action.export"), false, cx).opacity(0.45))
        })
}

fn data_table_header_text_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    tooltip: impl Into<SharedString>,
    icon_color: Hsla,
) -> Button {
    let label: SharedString = label.into();
    let tooltip: SharedString = tooltip.into();
    Button::new(id)
        .ghost()
        .xsmall()
        .h(px(20.))
        .min_w(px(20.))
        .p_0()
        .child(
            div()
                .text_size(ui_text_size(16.))
                .font_weight(gpui::FontWeight::BOLD)
                .line_height(ui_text_size(20.))
                .text_color(icon_color)
                .child(label),
        )
        .tooltip(tooltip)
}

fn data_table_header_icon_color(is_dark: bool) -> Hsla {
    if is_dark {
        rgb(0xc8ced8).into()
    } else {
        rgb(0x4f5865).into()
    }
}

fn data_table_header_menu_shell(cx: &mut Context<gpui_component::popover::PopoverState>) -> Div {
    let bg = if cx.theme().is_dark() {
        rgb(0x1f232a)
    } else {
        rgb(0xffffff)
    };
    let border = if cx.theme().is_dark() {
        rgb(0x3a414c)
    } else {
        rgb(0xd8dce2)
    };
    div()
        .occlude()
        .rounded(ComponentTheme::global(cx).radius)
        .border_1()
        .border_color(border)
        .bg(bg)
        .p(px(6.))
        .shadow(vec![box_shadow(
            px(0.),
            px(8.),
            px(18.),
            px(0.),
            hsla(0., 0., 0., 0.22),
        )])
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn data_table_header_menu_item(
    icon: &'static str,
    label: impl Into<SharedString>,
    selected: bool,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> Div {
    let label: SharedString = label.into();
    let dark = cx.theme().is_dark();
    let text = if dark { rgb(0xe6e8ec) } else { rgb(0x20242a) };
    let muted = if dark { rgb(0x98a2b3) } else { rgb(0x6b7280) };
    let row_bg = if selected {
        if dark { rgb(0x303844) } else { rgb(0xdfe4eb) }
    } else if dark {
        rgb(0x1f232a)
    } else {
        rgb(0xffffff)
    };
    let hover_bg = if dark { rgb(0x2a313b) } else { rgb(0xd7dce4) };
    div()
        .h(px(30.))
        .px_2()
        .rounded(ComponentTheme::global(cx).radius)
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .text_size(ui_text_size(13.))
        .text_color(text)
        .bg(row_bg)
        .hover(move |style| style.bg(hover_bg))
        .child(div().w(px(18.)).text_color(muted).child(icon))
        .child(label)
}

fn data_table_header_menu_item_icon(
    icon: AppIcon,
    label: impl Into<SharedString>,
    selected: bool,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> Div {
    let label: SharedString = label.into();
    let dark = cx.theme().is_dark();
    let text = if dark { rgb(0xe6e8ec) } else { rgb(0x20242a) };
    let row_bg = if selected {
        if dark { rgb(0x303844) } else { rgb(0xdfe4eb) }
    } else if dark {
        rgb(0x1f232a)
    } else {
        rgb(0xffffff)
    };
    let hover_bg = if dark { rgb(0x2a313b) } else { rgb(0xd7dce4) };
    div()
        .h(px(30.))
        .px_2()
        .rounded(ComponentTheme::global(cx).radius)
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .text_size(ui_text_size(13.))
        .text_color(text)
        .bg(row_bg)
        .hover(move |style| style.bg(hover_bg))
        .child(
            div()
                .w(px(18.))
                .flex()
                .items_center()
                .child(app_icon(icon, 14., if dark { rgb(0x98a2b3) } else { rgb(0x6b7280) })),
        )
        .child(label)
}

fn table_column_from_data_column(column: &GdbColumn) -> TableColumn {
    let mut name = column.name.clone();
    if column.primary_key {
        name.push_str("  PK");
    }

    TableColumn::new(column.name.clone(), name)
        .width(px(170.))
        .paddings(data_table_cell_padding())
        .movable(false)
}

fn data_table_cell_padding() -> gpui::Edges<Pixels> {
    gpui::Edges {
        top: px(5.),
        right: px(12.),
        bottom: px(5.),
        left: px(12.),
    }
}
