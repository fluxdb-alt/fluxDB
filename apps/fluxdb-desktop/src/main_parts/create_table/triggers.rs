fn create_table_triggers(
    tab_id: TabId,
    create: &CreateTableState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if create.database_kind == DatabaseKind::Dameng {
        return create_table_empty_state(tr("app.create_table.trigger.dameng_read_only"), colors)
            .debug_selector(|| "create-table-dameng-triggers-read-only".to_string());
    }
    let trigger_count = create.triggers.len();
    let mut rows = div().flex().flex_col();
    for (position, trigger) in create.triggers.iter().enumerate() {
        let name_input = this.create_table_input(
            CreateTableInputKey::TriggerName(tab_id, trigger.id),
            tr("app.create_table.trigger.name_placeholder"),
            &trigger.name,
            window,
            cx,
        );
        let timing_select = this.create_table_select(
            CreateTableSelectKey::TriggerTiming(tab_id, trigger.id),
            create_table_trigger_timing_options(),
            &trigger.timing,
            window,
            cx,
        );
        rows = rows.child(create_table_trigger_row(
            cx.entity().downgrade(),
            tab_id,
            trigger,
            name_input,
            timing_select,
            position,
            trigger_count,
            create.selected_trigger_id == Some(trigger.id),
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
        .child(create_table_trigger_header_row(colors))
        .child(
            div()
                .h(px(150.))
                .flex_none()
                .overflow_y_scrollbar()
                .child(rows),
        )
        .child(create_table_trigger_body_editor(
            tab_id, create, this, window, colors, cx,
        ))
}

fn create_table_trigger_header_row(colors: UiColors) -> Div {
    div()
        .h(px(32.))
        .flex_none()
        .flex_shrink_0()
        .bg(colors.panel_alt)
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .flex()
        .items_center()
        .child(create_table_trigger_header_cell(tr("app.create_table.header.name"), 240., colors))
        .child(create_table_trigger_header_cell(tr("app.create_table.header.trigger_timing"), 120., colors))
        .child(create_table_trigger_header_cell(tr("app.create_table.header.insert"), 80., colors))
        .child(create_table_trigger_header_cell(tr("app.create_table.header.update"), 80., colors))
        .child(create_table_trigger_header_cell(tr("app.create_table.header.delete"), 80., colors))
        .child(create_table_trigger_header_cell(tr("app.create_table.field.operations"), CREATE_TABLE_OPERATIONS_WIDTH, colors))
}

fn create_table_trigger_header_cell(
    label: impl Into<SharedString>,
    width: f32,
    colors: UiColors,
) -> Div {
    let label: SharedString = label.into();
    div()
        .debug_selector({
            let label = label.clone();
            move || format!("create-table-heading:{label}")
        })
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

fn create_table_trigger_row(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    trigger: &CreateTableTrigger,
    name_input: Entity<InputState>,
    timing_select: Entity<SelectState<SearchableVec<CreateTableSelectOption>>>,
    position: usize,
    trigger_count: usize,
    selected: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let select_view = view.clone();
    let trigger_id = trigger.id;
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
                    AppCommand::SelectCreateTableTrigger { tab_id, trigger_id },
                    cx,
                );
            });
            cx.stop_propagation();
        })
        .child(create_table_input_cell(name_input, 240., window, colors, cx))
        .child(create_table_select_cell_with_placeholder(
            timing_select,
            120.,
            "",
            window,
            colors,
            cx,
        ))
        .child(create_table_trigger_event_cell(
            view.clone(),
            tab_id,
            trigger.id,
            trigger.event.eq_ignore_ascii_case("INSERT"),
            CreateTableTriggerEvent::Insert,
            80.,
            colors,
        ))
        .child(create_table_trigger_event_cell(
            view.clone(),
            tab_id,
            trigger.id,
            trigger.event.eq_ignore_ascii_case("UPDATE"),
            CreateTableTriggerEvent::Update,
            80.,
            colors,
        ))
        .child(create_table_trigger_event_cell(
            view.clone(),
            tab_id,
            trigger.id,
            trigger.event.eq_ignore_ascii_case("DELETE"),
            CreateTableTriggerEvent::Delete,
            80.,
            colors,
        ))
        .child(create_table_trigger_operations_cell(
            view,
            tab_id,
            trigger.id,
            position > 0,
            position + 1 < trigger_count,
            colors,
        ))
}

fn create_table_trigger_event_cell(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    trigger_id: u64,
    checked: bool,
    event: CreateTableTriggerEvent,
    width: f32,
    colors: UiColors,
) -> Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .h_full()
        .border_l_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            let _ = view.update(cx, |this, cx| {
                this.dispatch(
                    AppCommand::SetCreateTableTriggerEvent {
                        tab_id,
                        trigger_id,
                        event,
                    },
                    cx,
                );
            });
            cx.stop_propagation();
        })
        .child(Checkbox::new(create_table_trigger_event_id(tab_id, trigger_id, event)).checked(checked))
}

fn create_table_trigger_timing_options() -> Vec<String> {
    ["BEFORE", "AFTER"].into_iter().map(str::to_string).collect()
}

fn create_table_trigger_body_editor(
    tab_id: TabId,
    create: &CreateTableState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(trigger) = create.selected_trigger() else {
        return div()
            .flex_1()
            .min_h(px(0.))
            .border_t_1()
            .border_color(colors.border)
            .flex()
            .items_center()
            .justify_center()
            .text_size(ui_text_size(13.))
            .text_color(colors.muted)
            .child(tr("app.create_table.trigger.add_definition"));
    };
    let body_input = this.create_table_input(
        CreateTableInputKey::TriggerBody(tab_id, trigger.id),
        "",
        &trigger.body,
        window,
        cx,
    );

    div()
        .flex_1()
        .min_h(px(0.))
        .border_t_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(34.))
                .flex_none()
                .px_3()
                .border_b_1()
                .border_color(colors.border_soft)
                .flex()
                .items_center()
                .gap_2()
                .text_size(ui_text_size(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(tr("app.create_table.definition"))
                // PG 触发器体指引用已存在函数，不是行内 SQL（后端 postgres 渲染 EXECUTE FUNCTION）。
                .when(create.database_kind == DatabaseKind::Postgres, |this| {
                    this.child(
                        div()
                            .text_size(ui_text_size(11.))
                            .font_weight(gpui::FontWeight::NORMAL)
                            .text_color(colors.muted)
                            .child(tr("app.create_table.trigger.postgres_function_hint")),
                    )
                }),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .p_3()
                .child(
                    Input::new(&body_input)
                        .appearance(false)
                        .bordered(false)
                        .focus_bordered(false)
                        .text_size(ui_text_size(12.))
                        .font_family(EDITOR_FONT)
                        .size_full(),
                ),
        )
}

#[derive(Clone, Copy)]
enum CreateTableTriggerAction {
    MoveUp,
    MoveDown,
    Remove,
}

fn create_table_trigger_operations_cell(
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    trigger_id: u64,
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
        .child(create_table_trigger_operation_button(
            tab_id,
            trigger_id,
            AppIcon::ChevronUp,
            can_move_up,
            CreateTableTriggerAction::MoveUp,
            view.clone(),
            colors,
        ))
        .child(create_table_trigger_operation_button(
            tab_id,
            trigger_id,
            AppIcon::ChevronDown,
            can_move_down,
            CreateTableTriggerAction::MoveDown,
            view.clone(),
            colors,
        ))
        .child(create_table_trigger_operation_button(
            tab_id,
            trigger_id,
            AppIcon::Trash,
            true,
            CreateTableTriggerAction::Remove,
            view,
            colors,
        ))
}

fn create_table_trigger_operation_button(
    tab_id: TabId,
    trigger_id: u64,
    icon: AppIcon,
    enabled: bool,
    action: CreateTableTriggerAction,
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
                CreateTableTriggerAction::MoveUp => AppCommand::MoveCreateTableTriggerUp {
                    tab_id,
                    trigger_id,
                },
                CreateTableTriggerAction::MoveDown => AppCommand::MoveCreateTableTriggerDown {
                    tab_id,
                    trigger_id,
                },
                CreateTableTriggerAction::Remove => AppCommand::RemoveCreateTableTrigger {
                    tab_id,
                    trigger_id,
                },
            };
            let _ = view.update(cx, |this, cx| {
                this.dispatch(command, cx);
            });
            cx.stop_propagation();
        },
    )
}

fn create_table_trigger_event_id(
    tab_id: TabId,
    trigger_id: u64,
    event: CreateTableTriggerEvent,
) -> gpui::ElementId {
    let event = match event {
        CreateTableTriggerEvent::Insert => "insert",
        CreateTableTriggerEvent::Update => "update",
        CreateTableTriggerEvent::Delete => "delete",
    };
    gpui::ElementId::Name(format!("create-table-trigger-{event}-{}-{trigger_id}", tab_id.0).into())
}
