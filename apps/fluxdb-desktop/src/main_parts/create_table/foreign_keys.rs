#[derive(Clone)]
struct CreateTableForeignKeyRow {
    foreign_key: CreateTableForeignKey,
    name_input: Entity<InputState>,
    referenced_database_select: Entity<SelectState<SearchableVec<String>>>,
    referenced_table_select: Entity<SelectState<SearchableVec<String>>>,
    on_delete_select: Entity<SelectState<SearchableVec<String>>>,
    on_update_select: Entity<SelectState<SearchableVec<String>>>,
}

include!("foreign_keys/options.rs");
include!("foreign_keys/referenced_fields.rs");

fn create_table_foreign_keys(
    tab_id: TabId,
    create: &CreateTableState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let available_fields = create_table_available_index_fields(create);
    let database_options = create_table_reference_database_options(create, this.controller.state());
    let foreign_key_count = create.foreign_keys.len();
    let mut rows = div().flex().flex_col();
    for (position, foreign_key) in create.foreign_keys.iter().enumerate() {
        if !foreign_key.referenced_table.trim().is_empty()
            && matches!(
                &foreign_key.referenced_column_options,
                LoadState::NotLoaded | LoadState::Failed(_)
            )
        {
            this.start_create_table_reference_columns_load(tab_id, foreign_key.id, cx);
        }
        let table_options_loading =
            create_table_ensure_reference_table_options_load(tab_id, create, this, foreign_key, cx);
        let table_options =
            create_table_reference_table_options(create, this.controller.state(), foreign_key);
        let row = CreateTableForeignKeyRow {
            foreign_key: foreign_key.clone(),
            name_input: this.create_table_input(
                CreateTableInputKey::ForeignKeyName(tab_id, foreign_key.id),
                "外键名",
                &foreign_key.name,
                window,
                cx,
            ),
            referenced_database_select: this.create_table_select(
                CreateTableSelectKey::ForeignKeyReferencedDatabase(tab_id, foreign_key.id),
                database_options.clone(),
                &foreign_key.referenced_database,
                window,
                cx,
            ),
            referenced_table_select: this.create_table_select(
                CreateTableSelectKey::ForeignKeyReferencedTable(tab_id, foreign_key.id),
                table_options,
                &foreign_key.referenced_table,
                window,
                cx,
            ),
            on_delete_select: this.create_table_select(
                CreateTableSelectKey::ForeignKeyOnDelete(tab_id, foreign_key.id),
                create_table_foreign_key_action_options(),
                &foreign_key.on_delete,
                window,
                cx,
            ),
            on_update_select: this.create_table_select(
                CreateTableSelectKey::ForeignKeyOnUpdate(tab_id, foreign_key.id),
                create_table_foreign_key_action_options(),
                &foreign_key.on_update,
                window,
                cx,
            ),
        };
        rows = rows.child(create_table_foreign_key_row(
            cx.entity().downgrade(),
            tab_id,
            row,
            available_fields.clone(),
            position,
            foreign_key_count,
            create.selected_foreign_key_id == Some(foreign_key.id),
            table_options_loading,
            window,
            colors,
            cx,
        ));
    }

    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(create_table_foreign_key_header_row(colors))
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .child(rows),
        )
}

fn create_table_foreign_key_header_row(colors: UiColors) -> Div {
    div()
        .h(px(32.))
        .flex_none()
        .flex_shrink_0()
        .bg(colors.panel_alt)
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .flex()
        .items_center()
        .child(create_table_field_header_cell("名称", 170., colors))
        .child(create_table_field_header_cell("字段", 230., colors))
        .child(create_table_field_header_cell("目标数据库", 150., colors))
        .child(create_table_field_header_cell("目标表", 160., colors))
        .child(create_table_field_header_cell("目标字段", 160., colors))
        .child(create_table_field_header_cell("删除时", 110., colors))
        .child(create_table_field_header_cell("更新时", 110., colors))
        .child(create_table_field_header_cell(
            "操作",
            CREATE_TABLE_OPERATIONS_WIDTH,
            colors,
        ))
}

fn create_table_ensure_reference_table_options_load(
    tab_id: TabId,
    create: &CreateTableState,
    this: &mut NavicatMain,
    foreign_key: &CreateTableForeignKey,
    cx: &mut Context<NavicatMain>,
) -> bool {
    let database = create_table_foreign_key_effective_database(create, foreign_key);
    if database.trim().is_empty() {
        return false;
    }
    let Some(database_path) =
        create_table_reference_database_path(this.controller.state(), tab_id, &database)
    else {
        return false;
    };
    let database_key = database_tree_key(database_path.connection_id, &database);
    if this.loading_databases.contains(&database_key) {
        return true;
    }
    if this.loaded_database_children.contains(&database_key) {
        return false;
    }
    let already_has_children = this
        .controller
        .state()
        .connections
        .iter()
        .find(|connection| connection.config.id == create.connection_id)
        .is_some_and(|connection| connection_has_loaded_children(connection, &database));
    if already_has_children {
        return false;
    }

    this.load_database_children(database_path, database_key, cx);
    true
}

fn create_table_foreign_key_row(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    row: CreateTableForeignKeyRow,
    available_fields: Vec<String>,
    position: usize,
    foreign_key_count: usize,
    selected: bool,
    table_options_loading: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let select_view = view.clone();
    let foreign_key_id = row.foreign_key.id;
    div()
        .h(px(34.))
        .flex_none()
        .flex_shrink_0()
        .bg(if selected { colors.hover } else { colors.panel_bg })
        .flex()
        .items_center()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            let _ = select_view.update(cx, |this, cx| {
                this.dispatch(
                    AppCommand::SelectCreateTableForeignKey {
                        tab_id,
                        foreign_key_id,
                    },
                    cx,
                );
            });
            cx.stop_propagation();
        })
        .child(create_table_input_cell(row.name_input, 170., window, colors, cx))
        .child(create_table_foreign_key_fields_cell(
            view.clone(),
            tab_id,
            row.foreign_key.clone(),
            available_fields,
            230.,
            colors,
        ))
        .child(create_table_select_cell_with_placeholder(
            row.referenced_database_select,
            150.,
            "当前库",
            window,
            colors,
            cx,
        ))
        .child(create_table_select_cell_with_loading(
            row.referenced_table_select,
            160.,
            if table_options_loading {
                "加载中"
            } else {
                "选择表"
            },
            table_options_loading,
            window,
            colors,
            cx,
        ))
        .child(create_table_foreign_key_referenced_fields_cell(
            view.clone(),
            tab_id,
            row.foreign_key.clone(),
            160.,
            colors,
        ))
        .child(create_table_select_cell_with_placeholder(
            row.on_delete_select,
            110.,
            "",
            window,
            colors,
            cx,
        ))
        .child(create_table_select_cell_with_placeholder(
            row.on_update_select,
            110.,
            "",
            window,
            colors,
            cx,
        ))
        .child(create_table_foreign_key_operations_cell(
            view,
            tab_id,
            row.foreign_key.id,
            position > 0,
            position + 1 < foreign_key_count,
            colors,
        ))
}

fn create_table_foreign_key_fields_cell(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    foreign_key: CreateTableForeignKey,
    available_fields: Vec<String>,
    width: f32,
    colors: UiColors,
) -> Div {
    let summary = create_table_foreign_key_fields_summary(&foreign_key);
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
                gpui::ElementId::Name(format!("create-table-fk-fields-{}", tab_id.0).into()),
                foreign_key.id.to_string(),
            ))
            .appearance(false)
            .anchor(Anchor::TopRight)
            .trigger(
                Button::new((
                    gpui::ElementId::Name(
                        format!("create-table-fk-fields-trigger-{}", tab_id.0).into(),
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
                                    this.create_table_foreign_key_fields_draft =
                                        Some(CreateTableForeignKeyFieldsDraft {
                                            tab_id,
                                            foreign_key_id: draft_foreign_key.id,
                                            columns: draft_foreign_key.columns.clone(),
                                            selected_field: draft_foreign_key.columns.first().cloned(),
                                        });
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
                                    "选择字段".to_string()
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
                create_table_foreign_key_fields_popover(
                    view.clone(),
                    tab_id,
                    foreign_key.clone(),
                    available_fields.clone(),
                    colors,
                    window,
                    cx,
                )
            }),
        )
}

fn create_table_foreign_key_fields_popover(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    foreign_key: CreateTableForeignKey,
    available_fields: Vec<String>,
    colors: UiColors,
    _window: &mut Window,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> Div {
    let popover = cx.entity();
    let draft = create_table_foreign_key_fields_draft(
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
                        .child("选择字段"),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(format!("已选 {} 个", draft.columns.len())),
                ),
        )
        .child(create_table_foreign_key_fields_table(
            view.clone(),
            tab_id,
            foreign_key.clone(),
            available_fields,
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
                                            .create_table_foreign_key_fields_draft
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
                                            .create_table_foreign_key_fields_draft
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
                                        this.create_table_foreign_key_fields_draft = None;
                                        cx.notify();
                                    });
                                    cancel_popover
                                        .update(cx, |state, cx| state.dismiss(window, cx));
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
                                        create_table_apply_foreign_key_fields_draft(
                                            this,
                                            tab_id,
                                            &foreign_key,
                                            cx,
                                        );
                                    });
                                    confirm_popover
                                        .update(cx, |state, cx| state.dismiss(window, cx));
                                    cx.stop_propagation();
                                }
                            },
                        )),
                ),
        )
}

fn create_table_foreign_key_fields_table(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    foreign_key: CreateTableForeignKey,
    available_fields: Vec<String>,
    draft: CreateTableForeignKeyFieldsDraft,
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
        .child(create_table_foreign_key_fields_header(colors));

    let mut list = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col();

    if available_fields.is_empty() {
        list = list.child(create_table_foreign_key_fields_empty(
            "请先填写字段名",
            colors,
        ));
    } else {
        for field in create_table_ordered_foreign_key_field_options(&draft.columns, available_fields)
        {
            list = list.child(create_table_foreign_key_field_option(
                view.clone(),
                tab_id,
                foreign_key.id,
                draft.clone(),
                field,
                colors,
            ));
        }
    }

    table = table.child(list);
    table
}

fn create_table_foreign_key_fields_header(colors: UiColors) -> Div {
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

fn create_table_foreign_key_field_option(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    foreign_key_id: u64,
    draft: CreateTableForeignKeyFieldsDraft,
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
                    .create_table_foreign_key_fields_draft
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
                                .create_table_foreign_key_fields_draft
                                .as_mut()
                                .filter(|draft| {
                                    draft.tab_id == tab_id && draft.foreign_key_id == foreign_key_id
                                })
                            {
                                if let Some(column_index) =
                                    draft.columns.iter().position(|column| column == &checkbox_field)
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
                        gpui::ElementId::Name(format!("create-table-fk-field-{}", tab_id.0).into()),
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

fn create_table_foreign_key_fields_draft(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    foreign_key_id: u64,
    foreign_key: &CreateTableForeignKey,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> CreateTableForeignKeyFieldsDraft {
    view.upgrade()
        .and_then(|view| view.read(cx).create_table_foreign_key_fields_draft.clone())
        .filter(|draft| draft.tab_id == tab_id && draft.foreign_key_id == foreign_key_id)
        .unwrap_or_else(|| CreateTableForeignKeyFieldsDraft {
            tab_id,
            foreign_key_id,
            columns: foreign_key.columns.clone(),
            selected_field: foreign_key.columns.first().cloned(),
        })
}

fn create_table_ordered_foreign_key_field_options(
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

fn create_table_apply_foreign_key_fields_draft(
    this: &mut NavicatMain,
    tab_id: TabId,
    foreign_key: &CreateTableForeignKey,
    cx: &mut Context<NavicatMain>,
) {
    let Some(draft) = this
        .create_table_foreign_key_fields_draft
        .take()
        .filter(|draft| draft.tab_id == tab_id && draft.foreign_key_id == foreign_key.id)
    else {
        return;
    };

    if draft.columns == foreign_key.columns {
        cx.notify();
        return;
    }

    for column_index in (0..foreign_key.columns.len()).rev() {
        this.dispatch(
            AppCommand::RemoveCreateTableForeignKeyColumn {
                tab_id,
                foreign_key_id: foreign_key.id,
                column_index,
            },
            cx,
        );
    }
    for (column_index, value) in draft.columns.iter().enumerate() {
        this.dispatch(
            AppCommand::AddCreateTableForeignKeyColumn {
                tab_id,
                foreign_key_id: foreign_key.id,
            },
            cx,
        );
        this.dispatch(
            AppCommand::SetCreateTableForeignKeyColumn {
                tab_id,
                foreign_key_id: foreign_key.id,
                column_index,
                value: value.clone(),
            },
            cx,
        );
    }
    this.create_table_foreign_key_field_selection =
        (!draft.columns.is_empty()).then_some(CreateTableForeignKeyFieldSelectionKey {
            tab_id,
            foreign_key_id: foreign_key.id,
            column_index: draft.columns.len() - 1,
            kind: CreateTableForeignKeyFieldSelectionKind::Local,
        });
    cx.notify();
}

#[derive(Clone, Copy)]
enum CreateTableForeignKeyAction {
    MoveUp,
    MoveDown,
    Remove,
}

fn create_table_foreign_key_operations_cell(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    foreign_key_id: u64,
    can_move_up: bool,
    can_move_down: bool,
    colors: UiColors,
) -> Div {
    div()
        .w(px(CREATE_TABLE_OPERATIONS_WIDTH))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_2()
        .flex()
        .items_center()
        .gap_1()
        .child(create_table_foreign_key_operation_button(
            tab_id,
            foreign_key_id,
            AppIcon::ChevronUp,
            can_move_up,
            CreateTableForeignKeyAction::MoveUp,
            view.clone(),
            colors,
        ))
        .child(create_table_foreign_key_operation_button(
            tab_id,
            foreign_key_id,
            AppIcon::ChevronDown,
            can_move_down,
            CreateTableForeignKeyAction::MoveDown,
            view.clone(),
            colors,
        ))
        .child(create_table_foreign_key_operation_button(
            tab_id,
            foreign_key_id,
            AppIcon::Trash,
            true,
            CreateTableForeignKeyAction::Remove,
            view,
            colors,
        ))
}

fn create_table_foreign_key_operation_button(
    tab_id: TabId,
    foreign_key_id: u64,
    icon: AppIcon,
    enabled: bool,
    action: CreateTableForeignKeyAction,
    view: WeakEntity<NavicatMain>,
    colors: UiColors,
) -> Div {
    let color = if enabled { colors.text } else { colors.muted };
    let button = div()
        .size(px(26.))
        .rounded(colors.radius)
        .flex()
        .items_center()
        .justify_center()
        .opacity(if enabled { 1.0 } else { 0.42 })
        .hover(move |style| if enabled { style.bg(colors.hover) } else { style })
        .child(app_icon(icon, 15., color));

    if !enabled {
        return button;
    }

    button.cursor_pointer().on_mouse_down(MouseButton::Left, move |_, _, cx| {
        let command = match action {
            CreateTableForeignKeyAction::MoveUp => AppCommand::MoveCreateTableForeignKeyUp {
                tab_id,
                foreign_key_id,
            },
            CreateTableForeignKeyAction::MoveDown => AppCommand::MoveCreateTableForeignKeyDown {
                tab_id,
                foreign_key_id,
            },
            CreateTableForeignKeyAction::Remove => AppCommand::RemoveCreateTableForeignKey {
                tab_id,
                foreign_key_id,
            },
        };
        let _ = view.update(cx, |this, cx| {
            this.dispatch(command, cx);
        });
        cx.stop_propagation();
    })
}
