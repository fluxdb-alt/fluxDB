#[derive(Clone)]
struct CreateTableIndexFieldEditorRow {
    index_id: u64,
    column_index: usize,
    name: String,
    sub_part_input: Entity<InputState>,
    sort_order: String,
}

fn create_table_indexes(
    tab_id: TabId,
    create: &CreateTableState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let available_fields = create_table_available_index_fields(create);
    let mut rows = div().flex().flex_col();
    if let Some(primary_fields) = create_table_primary_index_fields(create) {
        rows = rows.child(create_table_primary_index_row(primary_fields, colors));
    }

    let index_count = create.indexes.len();
    for (position, index) in create.indexes.iter().enumerate() {
        let field_rows = index
            .columns
            .iter()
            .enumerate()
            .map(|(column_index, column)| CreateTableIndexFieldEditorRow {
                index_id: index.id,
                column_index,
                name: column.name.clone(),
                sub_part_input: this.create_table_input(
                    CreateTableInputKey::IndexColumnSubPart(tab_id, index.id, column_index),
                    "子部分",
                    &column.sub_part,
                    window,
                    cx,
                ),
                sort_order: column.sort_order.clone(),
            })
            .collect::<Vec<_>>();
        let name_input = this.create_table_input(
            CreateTableInputKey::IndexName(tab_id, index.id),
            "索引名",
            &index.name,
            window,
            cx,
        );
        let type_select = this.create_table_select(
            CreateTableSelectKey::IndexType(tab_id, index.id),
            create_table_index_type_options(),
            &index.index_type,
            window,
            cx,
        );
        let method_select = this.create_table_select(
            CreateTableSelectKey::IndexMethod(tab_id, index.id),
            create_table_index_method_options_for_type(&index.index_type),
            &index.index_method,
            window,
            cx,
        );
        let comment_input = this.create_table_input(
            CreateTableInputKey::IndexComment(tab_id, index.id),
            "注释",
            &index.comment,
            window,
            cx,
        );
        rows = rows.child(create_table_index_row(
            cx.entity().downgrade(),
            tab_id,
            index,
            field_rows,
            available_fields.clone(),
            name_input,
            type_select,
            method_select,
            comment_input,
            position,
            index_count,
            create.selected_index_id == Some(index.id),
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
        .child(create_table_index_header_row(colors))
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .child(rows),
        )
}

fn create_table_index_header_row(colors: UiColors) -> Div {
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
        .child(create_table_index_header_cell("名称", 220., colors))
        .child(create_table_index_header_cell("字段", 280., colors))
        .child(create_table_index_header_cell("索引类型", 110., colors))
        .child(create_table_index_header_cell("索引方法", 110., colors))
        .child(create_table_index_header_cell("注释", 180., colors))
        .child(create_table_index_header_cell("操作", CREATE_TABLE_OPERATIONS_WIDTH, colors))
}

fn create_table_index_header_cell(label: &'static str, width: f32, colors: UiColors) -> Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_3()
        .flex()
        .items_center()
        .child(label)
}

fn create_table_primary_index_row(fields: String, colors: UiColors) -> Div {
    div()
        .h(px(34.))
        .flex_none()
        .flex_shrink_0()
        .bg(colors.panel_bg)
        .flex()
        .items_center()
        .child(create_table_readonly_cell("PRIMARY", 220., true, colors))
        .child(create_table_readonly_cell(fields, 280., false, colors))
        .child(create_table_readonly_cell("PRIMARY", 110., false, colors))
        .child(create_table_readonly_cell("BTREE", 110., false, colors))
        .child(create_table_readonly_cell("", 180., false, colors))
        .child(create_table_readonly_cell("", CREATE_TABLE_OPERATIONS_WIDTH, false, colors))
}

fn create_table_index_row(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    index: &CreateTableIndex,
    field_rows: Vec<CreateTableIndexFieldEditorRow>,
    available_fields: Vec<String>,
    name_input: Entity<InputState>,
    type_select: Entity<SelectState<SearchableVec<String>>>,
    method_select: Entity<SelectState<SearchableVec<String>>>,
    comment_input: Entity<InputState>,
    position: usize,
    index_count: usize,
    selected: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let select_view = view.clone();
    let index_id = index.id;
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
                this.dispatch(AppCommand::SelectCreateTableIndex { tab_id, index_id }, cx);
            });
            cx.stop_propagation();
        })
        .child(create_table_input_cell(name_input, 220., window, colors, cx))
        .child(create_table_index_fields_cell(
            view.clone(),
            tab_id,
            index.clone(),
            field_rows,
            available_fields,
            280.,
            colors,
        ))
        .child(create_table_select_cell_with_placeholder(
            type_select,
            110.,
            "",
            window,
            colors,
            cx,
        ))
        .child(create_table_select_cell_with_placeholder_enabled(
            method_select,
            110.,
            "",
            create_table_index_type_supports_method(&index.index_type),
            window,
            colors,
            cx,
        ))
        .child(create_table_input_cell(comment_input, 180., window, colors, cx))
        .child(create_table_index_operations_cell(
            view,
            tab_id,
            index.id,
            position > 0,
            position + 1 < index_count,
            colors,
        ))
}

#[derive(Clone, Copy)]
enum CreateTableIndexAction {
    MoveUp,
    MoveDown,
    Remove,
}

fn create_table_index_operations_cell(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    index_id: u64,
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
        .child(create_table_index_operation_button(
            tab_id,
            index_id,
            AppIcon::ChevronUp,
            can_move_up,
            CreateTableIndexAction::MoveUp,
            view.clone(),
            colors,
        ))
        .child(create_table_index_operation_button(
            tab_id,
            index_id,
            AppIcon::ChevronDown,
            can_move_down,
            CreateTableIndexAction::MoveDown,
            view.clone(),
            colors,
        ))
        .child(create_table_index_operation_button(
            tab_id,
            index_id,
            AppIcon::Trash,
            true,
            CreateTableIndexAction::Remove,
            view,
            colors,
        ))
}

fn create_table_index_operation_button(
    tab_id: TabId,
    index_id: u64,
    icon: AppIcon,
    enabled: bool,
    action: CreateTableIndexAction,
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

    button.cursor_pointer().on_mouse_down(
        MouseButton::Left,
        move |_, _, cx| {
            let command = match action {
                CreateTableIndexAction::MoveUp => AppCommand::MoveCreateTableIndexUp {
                    tab_id,
                    index_id,
                },
                CreateTableIndexAction::MoveDown => AppCommand::MoveCreateTableIndexDown {
                    tab_id,
                    index_id,
                },
                CreateTableIndexAction::Remove => AppCommand::RemoveCreateTableIndex {
                    tab_id,
                    index_id,
                },
            };
            let _ = view.update(cx, |this, cx| {
                this.dispatch(command, cx);
            });
            cx.stop_propagation();
        },
    )
}
