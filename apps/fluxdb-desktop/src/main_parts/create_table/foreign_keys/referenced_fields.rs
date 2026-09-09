fn create_table_foreign_key_referenced_fields_cell(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    foreign_key: CreateTableForeignKey,
    width: f32,
    colors: UiColors,
) -> Div {
    let summary = create_table_foreign_key_referenced_fields_summary(&foreign_key);
    div()
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_2()
        .flex()
        .items_center()
        .child(
            Popover::new((
                gpui::ElementId::Name(format!("create-table-fk-ref-fields-{}", tab_id.0).into()),
                foreign_key.id.to_string(),
            ))
            .appearance(false)
            .anchor(Anchor::TopRight)
            .trigger(
                Button::new((
                    gpui::ElementId::Name(
                        format!("create-table-fk-ref-fields-trigger-{}", tab_id.0).into(),
                    ),
                    foreign_key.id.to_string(),
                ))
                .ghost()
                .xsmall()
                .w(px(width - 18.))
                .h(px(CREATE_TABLE_INPUT_HEIGHT))
                .p_0()
                .child(
                    div()
                        .size_full()
                        .w(px(width - 18.))
                        .h(px(CREATE_TABLE_INPUT_HEIGHT))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.input_bg)
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .justify_between()
                        .cursor_pointer()
                        .hover(move |style| {
                            style.border_color(create_table_input_hover_border_color(false, colors))
                        })
                        .on_mouse_down(MouseButton::Left, {
                            let draft_view = view.clone();
                            let draft_foreign_key = foreign_key.clone();
                            move |_, _, cx| {
                                let _ = draft_view.update(cx, |this, cx| {
                                    this.create_table_foreign_key_referenced_fields_draft = Some(
                                        CreateTableForeignKeyReferencedFieldsDraft {
                                            tab_id,
                                            foreign_key_id: draft_foreign_key.id,
                                            columns: draft_foreign_key.referenced_columns.clone(),
                                            selected_field: draft_foreign_key
                                                .referenced_columns
                                                .first()
                                                .cloned(),
                                        },
                                    );
                                    cx.notify();
                                });
                            }
                        })
                        .child(
                            div()
                                .min_w(px(0.))
                                .flex_1()
                                .px_2()
                                .text_size(px(13.))
                                .text_color(colors.text)
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(if summary.is_empty() {
                                    "选择目标字段".to_string()
                                } else {
                                    summary
                                }),
                        )
                        .child(
                            div()
                                .w(px(24.))
                                .h_full()
                                .border_l_1()
                                .border_color(colors.border_soft)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(app_icon(AppIcon::ChevronDown, 13., colors.muted)),
                        ),
                ),
            )
            .content(move |_, window, cx| {
                create_table_foreign_key_referenced_fields_popover(
                    view.clone(),
                    tab_id,
                    foreign_key.clone(),
                    colors,
                    window,
                    cx,
                )
            }),
        )
}

fn create_table_foreign_key_referenced_fields_popover(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    foreign_key: CreateTableForeignKey,
    colors: UiColors,
    _window: &mut Window,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> Div {
    let popover = cx.entity();
    let options = create_table_foreign_key_referenced_field_options(&foreign_key);
    let draft = create_table_foreign_key_referenced_fields_draft(
        view.clone(),
        tab_id,
        foreign_key.id,
        &foreign_key,
        cx,
    );
    let selected_column_index = draft
        .selected_field
        .as_ref()
        .and_then(|field| draft.columns.iter().position(|column| column == field));
    let can_move_up = selected_column_index.is_some_and(|column_index| column_index > 0);
    let can_move_down =
        selected_column_index.is_some_and(|column_index| column_index + 1 < draft.columns.len());

    div()
        .w(px(360.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow_md()
        .overflow_hidden()
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(
            div()
                .h(px(32.))
                .px_3()
                .border_b_1()
                .border_color(colors.border_soft)
                .bg(colors.panel_alt)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("选择目标字段"),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(format!("已选 {} 个", draft.columns.len())),
                ),
        )
        .child(create_table_foreign_key_referenced_fields_table(
            view.clone(),
            tab_id,
            foreign_key.clone(),
            options,
            draft,
            colors,
        ))
        .child(
            div()
                .h(px(40.))
                .px_2()
                .border_t_1()
                .border_color(colors.border_soft)
                .bg(colors.panel_alt)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(create_table_index_field_icon_button(
                            AppIcon::ChevronUp,
                            "上移字段",
                            can_move_up,
                            colors,
                        )
                        .on_mouse_down(MouseButton::Left, {
                            let move_view = view.clone();
                            move |_, _, cx| {
                                if let Some(column_index) = selected_column_index {
                                    let _ = move_view.update(cx, |this, cx| {
                                        if let Some(draft) = this
                                            .create_table_foreign_key_referenced_fields_draft
                                            .as_mut()
                                            .filter(|draft| {
                                                draft.tab_id == tab_id
                                                    && draft.foreign_key_id == foreign_key.id
                                                    && column_index > 0
                                                    && column_index < draft.columns.len()
                                            })
                                        {
                                            draft.columns.swap(column_index - 1, column_index);
                                            cx.notify();
                                        }
                                    });
                                }
                                cx.stop_propagation();
                            }
                        }))
                        .child(create_table_index_field_icon_button(
                            AppIcon::ChevronDown,
                            "下移字段",
                            can_move_down,
                            colors,
                        )
                        .on_mouse_down(MouseButton::Left, {
                            let move_view = view.clone();
                            move |_, _, cx| {
                                if let Some(column_index) = selected_column_index {
                                    let _ = move_view.update(cx, |this, cx| {
                                        if let Some(draft) = this
                                            .create_table_foreign_key_referenced_fields_draft
                                            .as_mut()
                                            .filter(|draft| {
                                                draft.tab_id == tab_id
                                                    && draft.foreign_key_id == foreign_key.id
                                                    && column_index + 1 < draft.columns.len()
                                            })
                                        {
                                            draft.columns.swap(column_index, column_index + 1);
                                            cx.notify();
                                        }
                                    });
                                }
                                cx.stop_propagation();
                            }
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(create_table_popover_text_button("取消", colors).on_mouse_down(
                            MouseButton::Left,
                            {
                            let cancel_view = view.clone();
                            let cancel_popover = popover.clone();
                            move |_, window, cx| {
                                let _ = cancel_view.update(cx, |this, cx| {
                                    this.create_table_foreign_key_referenced_fields_draft = None;
                                    cx.notify();
                                });
                                cancel_popover.update(cx, |state, cx| state.dismiss(window, cx));
                                cx.stop_propagation();
                            }
                            },
                        ))
                        .child(create_table_popover_text_button("确定", colors).on_mouse_down(
                            MouseButton::Left,
                            {
                            let confirm_view = view.clone();
                            let confirm_popover = popover.clone();
                            move |_, window, cx| {
                                let _ = confirm_view.update(cx, |this, cx| {
                                    create_table_apply_foreign_key_referenced_fields_draft(
                                        this,
                                        tab_id,
                                        &foreign_key,
                                        cx,
                                    );
                                });
                                confirm_popover.update(cx, |state, cx| state.dismiss(window, cx));
                                cx.stop_propagation();
                            }
                            },
                        )),
                ),
        )
}

fn create_table_foreign_key_referenced_fields_table(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    foreign_key: CreateTableForeignKey,
    options: Vec<String>,
    draft: CreateTableForeignKeyReferencedFieldsDraft,
    colors: UiColors,
) -> Div {
    let mut table = div()
        .h(px(244.))
        .mx_2()
        .my_2()
        .border_1()
        .border_color(colors.border_soft)
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(create_table_foreign_key_referenced_fields_header(colors));

    let mut list = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col();

    if foreign_key.referenced_table.trim().is_empty() {
        list = list.child(create_table_foreign_key_fields_empty(
            "请先选择目标表",
            colors,
        ));
    } else {
        match &foreign_key.referenced_column_options {
            LoadState::NotLoaded | LoadState::Loading => {
                list = list.child(create_table_foreign_key_fields_empty("正在加载字段", colors));
            }
            LoadState::Failed(error) => {
                list = list.child(create_table_foreign_key_fields_empty(
                    format!("加载失败：{}", error.message),
                    colors,
                ));
            }
            LoadState::Loaded(_) if options.is_empty() => {
                list = list.child(create_table_foreign_key_fields_empty("目标表暂无字段", colors));
            }
            LoadState::Loaded(_) => {
                for field in create_table_ordered_referenced_field_options(&draft.columns, options)
                {
                    list = list.child(create_table_foreign_key_referenced_field_option(
                        view.clone(),
                        tab_id,
                        foreign_key.id,
                        draft.clone(),
                        field,
                        colors,
                    ));
                }
            }
        }
    }

    table = table.child(list);
    table
}

fn create_table_foreign_key_referenced_fields_header(colors: UiColors) -> Div {
    div()
        .h(px(30.))
        .flex_none()
        .bg(colors.panel_alt)
        .border_b_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(
            div()
                .w(px(42.))
                .h_full()
                .border_r_1()
                .border_color(colors.border_soft),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h_full()
                .px_2()
                .flex()
                .items_center()
                .child("名称"),
        )
}

fn create_table_foreign_key_referenced_field_option(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    foreign_key_id: u64,
    draft: CreateTableForeignKeyReferencedFieldsDraft,
    field: String,
    colors: UiColors,
) -> Div {
    let selected_index = draft.columns.iter().position(|column| column == &field);
    let selected = selected_index.is_some();
    let is_active = draft
        .selected_field
        .as_ref()
        .is_some_and(|selected_field| selected_field == &field);
    let field_value = field.clone();
    let row_view = view.clone();
    let checkbox_view = view;
    div()
        .h(px(28.))
        .flex_none()
        .flex()
        .items_center()
        .text_size(px(13.))
        .text_color(if selected { colors.text } else { colors.muted })
        .bg(if is_active { colors.hover } else { colors.panel_bg })
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            let _ = row_view.update(cx, |this, cx| {
                if let Some(draft) = this
                    .create_table_foreign_key_referenced_fields_draft
                    .as_mut()
                    .filter(|draft| draft.tab_id == tab_id && draft.foreign_key_id == foreign_key_id)
                {
                    draft.selected_field = Some(field_value.clone());
                    cx.notify();
                }
            });
            cx.stop_propagation();
        })
        .child(
            div()
                .w(px(42.))
                .h_full()
                .border_r_1()
                .border_color(colors.border_soft)
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(MouseButton::Left, {
                    let checkbox_field = field.clone();
                    move |_, _, cx| {
                        let _ = checkbox_view.update(cx, |this, cx| {
                            if let Some(draft) = this
                                .create_table_foreign_key_referenced_fields_draft
                                .as_mut()
                                .filter(|draft| {
                                    draft.tab_id == tab_id && draft.foreign_key_id == foreign_key_id
                                })
                            {
                                if let Some(column_index) = draft
                                    .columns
                                    .iter()
                                    .position(|column| column == &checkbox_field)
                                {
                                    draft.columns.remove(column_index);
                                } else {
                                    draft.columns.push(checkbox_field.clone());
                                }
                                draft.selected_field = Some(checkbox_field.clone());
                                cx.notify();
                            }
                        });
                        cx.stop_propagation();
                    }
                })
                .child(
                    Checkbox::new((
                        gpui::ElementId::Name(
                            format!("create-table-fk-ref-field-dialog-{}", tab_id.0).into(),
                        ),
                        format!("{}-{}", foreign_key_id, field),
                    ))
                    .checked(selected),
                ),
        )
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .h_full()
                .px_2()
                .border_b_1()
                .border_color(colors.border_soft)
                .flex()
                .items_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(field),
        )
}

fn create_table_foreign_key_referenced_fields_draft(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    foreign_key_id: u64,
    foreign_key: &CreateTableForeignKey,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> CreateTableForeignKeyReferencedFieldsDraft {
    view.upgrade()
        .and_then(|view| {
            view.read(cx)
                .create_table_foreign_key_referenced_fields_draft
                .clone()
        })
        .filter(|draft| draft.tab_id == tab_id && draft.foreign_key_id == foreign_key_id)
        .unwrap_or_else(|| CreateTableForeignKeyReferencedFieldsDraft {
            tab_id,
            foreign_key_id,
            columns: foreign_key.referenced_columns.clone(),
            selected_field: foreign_key.referenced_columns.first().cloned(),
        })
}

fn create_table_ordered_referenced_field_options(
    selected_columns: &[String],
    options: Vec<String>,
) -> Vec<String> {
    let mut ordered = Vec::new();
    for column in selected_columns {
        if !ordered.iter().any(|field| field == column) {
            ordered.push(column.clone());
        }
    }
    for option in options {
        if !ordered.iter().any(|field| field == &option) {
            ordered.push(option);
        }
    }
    ordered
}

fn create_table_apply_foreign_key_referenced_fields_draft(
    this: &mut NavicatMain,
    tab_id: TabId,
    foreign_key: &CreateTableForeignKey,
    cx: &mut Context<NavicatMain>,
) {
    let Some(draft) = this
        .create_table_foreign_key_referenced_fields_draft
        .take()
        .filter(|draft| draft.tab_id == tab_id && draft.foreign_key_id == foreign_key.id)
    else {
        return;
    };

    if draft.columns == foreign_key.referenced_columns {
        cx.notify();
        return;
    }

    for column_index in (0..foreign_key.referenced_columns.len()).rev() {
        this.dispatch(
            AppCommand::RemoveCreateTableForeignKeyReferencedColumn {
                tab_id,
                foreign_key_id: foreign_key.id,
                column_index,
            },
            cx,
        );
    }
    for (column_index, value) in draft.columns.iter().enumerate() {
        this.dispatch(
            AppCommand::AddCreateTableForeignKeyReferencedColumn {
                tab_id,
                foreign_key_id: foreign_key.id,
            },
            cx,
        );
        this.dispatch(
            AppCommand::SetCreateTableForeignKeyReferencedColumn {
                tab_id,
                foreign_key_id: foreign_key.id,
                column_index,
                value: value.clone(),
            },
            cx,
        );
    }
    this.create_table_foreign_key_field_selection = (!draft.columns.is_empty()).then_some(
        CreateTableForeignKeyFieldSelectionKey {
            tab_id,
            foreign_key_id: foreign_key.id,
            column_index: draft.columns.len() - 1,
            kind: CreateTableForeignKeyFieldSelectionKind::Referenced,
        },
    );
    cx.notify();
}

fn create_table_foreign_key_fields_empty(text: impl Into<String>, colors: UiColors) -> Div {
    div()
        .h(px(30.))
        .px_3()
        .flex()
        .items_center()
        .text_size(px(13.))
        .text_color(colors.muted)
        .child(text.into())
}
