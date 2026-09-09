#[derive(Clone)]
struct CreateTableCheckRow {
    check: CreateTableCheck,
    name_input: Entity<InputState>,
    expression_input: Entity<InputState>,
    expression_editor_input: Entity<InputState>,
}

fn create_table_checks(
    tab_id: TabId,
    create: &CreateTableState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let check_count = create.checks.len();
    let not_enforced_supported =
        create.database_kind == DatabaseKind::MySql || create.database_kind == DatabaseKind::TiDb;
    let mut rows = div().flex().flex_col();
    for (position, check) in create.checks.iter().enumerate() {
        let row = CreateTableCheckRow {
            check: check.clone(),
            name_input: this.create_table_input(
                CreateTableInputKey::CheckName(tab_id, check.id),
                "检查名",
                &check.name,
                window,
                cx,
            ),
            expression_input: this.create_table_input(
                CreateTableInputKey::CheckExpression(tab_id, check.id),
                "例如 age >= 0",
                &check.expression,
                window,
                cx,
            ),
            expression_editor_input: this.create_table_multiline_input(
                CreateTableInputKey::CheckExpressionEditor(tab_id, check.id),
                "输入检查表达式...",
                &check.expression,
                8,
                window,
                cx,
            ),
        };
        rows = rows.child(create_table_check_row(
            cx.entity().downgrade(),
            tab_id,
            row,
            position,
            check_count,
            create.selected_check_id == Some(check.id),
            this.create_table_check_expression_editor_sizes
                .get(&(tab_id, check.id))
                .copied()
                .unwrap_or((
                    CREATE_TABLE_COMMENT_EDITOR_DEFAULT_WIDTH,
                    CREATE_TABLE_COMMENT_EDITOR_DEFAULT_HEIGHT,
                )),
            not_enforced_supported,
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
        .child(create_table_check_header_row(colors))
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .child(rows),
        )
}

fn create_table_check_header_row(colors: UiColors) -> Div {
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
        .child(create_table_field_header_cell("名称", 220., colors))
        .child(create_table_field_header_cell("检查", 420., colors))
        .child(create_table_field_header_cell("不强制实施", 120., colors))
        .child(create_table_field_header_cell(
            "操作",
            CREATE_TABLE_OPERATIONS_WIDTH,
            colors,
        ))
}

fn create_table_check_row(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    row: CreateTableCheckRow,
    position: usize,
    check_count: usize,
    selected: bool,
    editor_size: (f32, f32),
    not_enforced_supported: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let select_view = view.clone();
    let check_id = row.check.id;
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
                this.dispatch(AppCommand::SelectCreateTableCheck { tab_id, check_id }, cx);
            });
            cx.stop_propagation();
        })
        .child(create_table_input_cell(
            row.name_input,
            220.,
            window,
            colors,
            cx,
        ))
        .child(create_table_check_expression_cell(
            view.clone(),
            tab_id,
            row.check.id,
            row.expression_input,
            row.expression_editor_input,
            editor_size,
            420.,
            window,
            colors,
            cx,
        ))
        .child(create_table_check_not_enforced_cell(
            view.clone(),
            tab_id,
            row.check.id,
            row.check.not_enforced,
            not_enforced_supported,
            colors,
        ))
        .child(create_table_check_operations_cell(
            view,
            tab_id,
            row.check.id,
            position > 0,
            position + 1 < check_count,
            colors,
        ))
}

fn create_table_check_expression_cell<T: 'static>(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    check_id: u64,
    input: Entity<InputState>,
    editor_input: Entity<InputState>,
    editor_size: (f32, f32),
    width: f32,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<T>,
) -> Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_2()
        .flex()
        .items_center()
        .gap_1()
        .child(create_table_input_box(
            input,
            width - 48.,
            window,
            colors,
            cx,
        ))
        .child(create_table_check_expression_popover(
            view,
            tab_id,
            check_id,
            editor_input,
            editor_size,
            colors,
        ))
}

fn create_table_check_expression_popover(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    check_id: u64,
    editor_input: Entity<InputState>,
    editor_size: (f32, f32),
    colors: UiColors,
) -> Div {
    div().child(
        Popover::new((
            gpui::ElementId::Name(format!("create-table-check-popover-{}", tab_id.0).into()),
            check_id.to_string(),
        ))
        .appearance(false)
        .anchor(Anchor::TopRight)
        .trigger(
            Button::new((
                gpui::ElementId::Name(format!("create-table-check-trigger-{}", tab_id.0).into()),
                check_id.to_string(),
            ))
            .ghost()
            .xsmall()
            .w(px(26.))
            .h(px(CREATE_TABLE_INPUT_HEIGHT))
            .p_0()
            .child(app_icon(AppIcon::Maximize, 14., colors.muted)),
        )
        .content(move |_, window, cx| {
            let width = clamp_create_table_comment_editor_width(editor_size.0, window);
            let height = clamp_create_table_comment_editor_height(editor_size.1, window);
            let focused = editor_input.read(cx).focus_handle(cx).is_focused(window);
            if !focused {
                editor_input.read(cx).focus_handle(cx).focus(window, cx);
            }

            div()
                .relative()
                .w(px(width))
                .h(px(height))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow_md()
                .p_3()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex_none()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("编辑检查"),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(create_table_input_border_color(focused, colors))
                        .bg(colors.input_bg)
                        .overflow_hidden()
                        .hover(move |style| {
                            style.border_color(create_table_input_hover_border_color(
                                focused, colors,
                            ))
                        })
                        .child(
                            Input::new(&editor_input)
                                .appearance(false)
                                .focus_bordered(false)
                                .w_full()
                                .h_full()
                                .px_2()
                                .text_size(px(13.)),
                        ),
                )
                .child(create_table_check_expression_editor_resize_handle(
                    view.clone(),
                    tab_id,
                    check_id,
                    width,
                    height,
                    colors,
                    cx,
                ))
        }),
    )
}

fn create_table_check_expression_editor_resize_handle<T: 'static>(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    check_id: u64,
    width: f32,
    height: f32,
    colors: UiColors,
    cx: &mut Context<T>,
) -> impl IntoElement {
    let mouse_down_view = view.clone();
    let drag_move_view = view.clone();
    let mouse_up_view = view;
    div()
        .id(gpui::ElementId::Name(
            format!(
                "create-table-check-editor-resize-{}-{}",
                tab_id.0, check_id
            )
            .into(),
        ))
        .absolute()
        .right(px(5.))
        .bottom(px(5.))
        .size(px(16.))
        .cursor_nwse_resize()
        .rounded(colors.radius * 0.5)
        .opacity(0.7)
        .hover(move |style| style.bg(colors.hover).opacity(1.0))
        .child(
            div()
                .absolute()
                .right(px(3.))
                .bottom(px(3.))
                .w(px(7.))
                .h(px(7.))
                .border_r_1()
                .border_b_1()
                .border_color(colors.muted),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_, event: &MouseDownEvent, _, cx| {
                let _ = mouse_down_view.update(cx, |this, cx| {
                    this.create_table_check_expression_editor_resize_start =
                        Some(CreateTableCheckExpressionEditorResizeStart {
                            tab_id,
                            check_id,
                            x: f32::from(event.position.x),
                            y: f32::from(event.position.y),
                            width,
                            height,
                        });
                    cx.notify();
                });
                cx.stop_propagation();
            }),
        )
        .on_drag(CreateTableCommentEditorResizeDrag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_drag_move(cx.listener(
            move |_, event: &DragMoveEvent<CreateTableCommentEditorResizeDrag>, window, cx| {
                let _ = drag_move_view.update(cx, |this, cx| {
                    let Some(start) = this.create_table_check_expression_editor_resize_start else {
                        return;
                    };
                    if start.tab_id != tab_id || start.check_id != check_id {
                        return;
                    }
                    let next_width = clamp_create_table_comment_editor_width(
                        start.width + f32::from(event.event.position.x) - start.x,
                        window,
                    );
                    let next_height = clamp_create_table_comment_editor_height(
                        start.height + f32::from(event.event.position.y) - start.y,
                        window,
                    );
                    this.create_table_check_expression_editor_sizes
                        .insert((tab_id, check_id), (next_width, next_height));
                    cx.notify();
                });
                cx.stop_propagation();
            },
        ))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |_, _, _, cx| {
                let _ = mouse_up_view.update(cx, |this, cx| {
                    this.create_table_check_expression_editor_resize_start = None;
                    cx.notify();
                });
                cx.stop_propagation();
            }),
        )
}

fn create_table_check_not_enforced_cell(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    check_id: u64,
    checked: bool,
    enabled: bool,
    colors: UiColors,
) -> Div {
    let cell = div()
        .w(px(120.))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .px_3()
        .flex()
        .items_center()
        .opacity(if enabled { 1.0 } else { 0.45 })
        .child(Checkbox::new(create_table_check_checkbox_id(tab_id, check_id)).checked(checked));

    if !enabled {
        return cell;
    }

    cell.cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            let _ = view.update(cx, |this, cx| {
                this.dispatch(
                    AppCommand::ToggleCreateTableCheckNotEnforced { tab_id, check_id },
                    cx,
                );
            });
            cx.stop_propagation();
        })
}

#[derive(Clone, Copy)]
enum CreateTableCheckAction {
    MoveUp,
    MoveDown,
    Remove,
}

fn create_table_check_operations_cell(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    check_id: u64,
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
        .child(create_table_check_operation_button(
            tab_id,
            check_id,
            AppIcon::ChevronUp,
            can_move_up,
            CreateTableCheckAction::MoveUp,
            view.clone(),
            colors,
        ))
        .child(create_table_check_operation_button(
            tab_id,
            check_id,
            AppIcon::ChevronDown,
            can_move_down,
            CreateTableCheckAction::MoveDown,
            view.clone(),
            colors,
        ))
        .child(create_table_check_operation_button(
            tab_id,
            check_id,
            AppIcon::Trash,
            true,
            CreateTableCheckAction::Remove,
            view,
            colors,
        ))
}

fn create_table_check_operation_button(
    tab_id: TabId,
    check_id: u64,
    icon: AppIcon,
    enabled: bool,
    action: CreateTableCheckAction,
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
                CreateTableCheckAction::MoveUp => {
                    AppCommand::MoveCreateTableCheckUp { tab_id, check_id }
                }
                CreateTableCheckAction::MoveDown => {
                    AppCommand::MoveCreateTableCheckDown { tab_id, check_id }
                }
                CreateTableCheckAction::Remove => {
                    AppCommand::RemoveCreateTableCheck { tab_id, check_id }
                }
            };
            let _ = view.update(cx, |this, cx| {
                this.dispatch(command, cx);
            });
            cx.stop_propagation();
        },
    )
}

fn create_table_check_checkbox_id(tab_id: TabId, check_id: u64) -> gpui::ElementId {
    gpui::ElementId::Name(format!("create-table-check-not-enforced-{}-{check_id}", tab_id.0).into())
}
