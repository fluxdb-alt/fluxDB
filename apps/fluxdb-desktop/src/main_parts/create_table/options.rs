fn create_table_options(
    tab_id: TabId,
    create: &CreateTableState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if !matches!(create.database_kind, DatabaseKind::MySql | DatabaseKind::TiDb) {
        return create_table_empty_state("当前数据库暂无表级选项", colors);
    }

    let engine_select = this.create_table_select(
        CreateTableSelectKey::TableEngine(tab_id),
        create_table_inherit_select_options(create_table_engine_options()),
        &create.engine,
        window,
        cx,
    );
    let charset_select = this.create_table_select(
        CreateTableSelectKey::TableCharset(tab_id),
        create_table_inherit_select_options(create_database_charset_options()),
        &create.charset,
        window,
        cx,
    );
    let collation_charset = if create.charset.trim().is_empty() {
        "utf8mb4"
    } else {
        create.charset.trim()
    };
    let collation_select = this.create_table_select(
        CreateTableSelectKey::TableCollation(tab_id),
        create_table_inherit_select_options(create_database_collation_options(collation_charset)),
        &create.collation,
        window,
        cx,
    );
    let row_format_select = this.create_table_select(
        CreateTableSelectKey::TableRowFormat(tab_id),
        create_table_inherit_select_options(create_table_row_format_options()),
        &create.row_format,
        window,
        cx,
    );

    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .px_4()
        .py_4()
        .child(
            div().size_full().overflow_y_scrollbar().child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(create_table_table_option_select(
                        "引擎",
                        engine_select,
                        300.,
                        "引擎",
                        window,
                        colors,
                        cx,
                    ))
                    .child(create_table_table_option_input(
                        "表空间",
                        this.create_table_input(
                            CreateTableInputKey::TableTablespace(tab_id),
                            "表空间",
                            &create.tablespace,
                            window,
                            cx,
                        ),
                        300.,
                        window,
                        colors,
                        cx,
                    ))
                    .child(create_table_table_option_select(
                        "字符集",
                        charset_select,
                        300.,
                        "字符集",
                        window,
                        colors,
                        cx,
                    ))
                    .child(create_table_table_option_select(
                        "排序规则",
                        collation_select,
                        300.,
                        "排序规则",
                        window,
                        colors,
                        cx,
                    ))
                    .child(create_table_table_option_select(
                        "行格式",
                        row_format_select,
                        276.,
                        "行格式",
                        window,
                        colors,
                        cx,
                    ))
                    .child(create_table_table_option_input(
                        "平均行长度",
                        this.create_table_input(
                            CreateTableInputKey::TableAvgRowLength(tab_id),
                            "0",
                            &create.avg_row_length,
                            window,
                            cx,
                        ),
                        132.,
                        window,
                        colors,
                        cx,
                    ))
                    .child(create_table_table_option_input(
                        "最大行数",
                        this.create_table_input(
                            CreateTableInputKey::TableMaxRows(tab_id),
                            "0",
                            &create.max_rows,
                            window,
                            cx,
                        ),
                        132.,
                        window,
                        colors,
                        cx,
                    ))
                    .child(create_table_table_option_input(
                        "最小行数",
                        this.create_table_input(
                            CreateTableInputKey::TableMinRows(tab_id),
                            "0",
                            &create.min_rows,
                            window,
                            cx,
                        ),
                        132.,
                        window,
                        colors,
                        cx,
                    ))
                    .child(create_table_table_option_input(
                        "键块大小",
                        this.create_table_input(
                            CreateTableInputKey::TableKeyBlockSize(tab_id),
                            "0",
                            &create.key_block_size,
                            window,
                            cx,
                        ),
                        132.,
                        window,
                        colors,
                        cx,
                    )),
            ),
        )
}

fn create_table_partitions(
    tab_id: TabId,
    create: &CreateTableState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if !matches!(create.database_kind, DatabaseKind::MySql | DatabaseKind::TiDb) {
        create_table_empty_state("当前数据库暂无分区选项", colors)
    } else {
        let method_select = this.create_table_select(
            CreateTableSelectKey::TablePartitionMethod(tab_id),
            create_table_partition_method_options(),
            &create.partition_method,
            window,
            cx,
        );
        let expression_input = this.create_table_input(
            CreateTableInputKey::TablePartitionExpression(tab_id),
            "例如 TO_DAYS(created_at)",
            &create.partition_expression,
            window,
            cx,
        );
        let sql_input = this.create_table_multiline_input(
            CreateTableInputKey::TablePartitionSql(tab_id),
            "PARTITION BY RANGE (created_at)",
            &create.partition_sql,
            8,
            window,
            cx,
        );
        let view = cx.entity().downgrade();

        div()
            .flex_1()
            .min_h(px(0.))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px_3()
                    .py_3()
                    .border_b_1()
                    .border_color(colors.border_soft)
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.dispatch(
                                        AppCommand::ToggleCreateTablePartitionEnabled(tab_id),
                                        cx,
                                    );
                                });
                                cx.stop_propagation();
                            })
                            .child(
                                Checkbox::new(("create-table-partition-enabled", tab_id.0))
                                    .checked(create.partition_enabled),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(colors.text)
                                    .child("启用"),
                            ),
                    )
                    .child(create_table_table_option_select(
                        "方式",
                        method_select,
                        170.,
                        "分区方式",
                        window,
                        colors,
                        cx,
                    ))
                    .child(create_table_table_option_input(
                        "表达式",
                        expression_input,
                        420.,
                        window,
                        colors,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
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
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.text)
                            .child("定义"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.))
                            .p_3()
                            .child(
                                Input::new(&sql_input)
                                    .appearance(false)
                                    .bordered(false)
                                    .focus_bordered(false)
                                    .text_size(px(12.))
                                    .font_family(EDITOR_FONT)
                                    .size_full(),
                            ),
                    ),
            )
    }
}

fn create_table_partition_method_options() -> Vec<String> {
    ["RANGE", "LIST", "HASH", "KEY"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn create_table_table_option_input(
    label: &'static str,
    input: Entity<InputState>,
    width: f32,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(create_table_table_option_label(label, colors))
        .child(create_table_input_box(input, width, window, colors, cx))
}

fn create_table_table_option_select(
    label: &'static str,
    select: Entity<SelectState<SearchableVec<String>>>,
    width: f32,
    placeholder: &'static str,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(create_table_table_option_label(label, colors))
        .child(create_table_option_select_box(
            select,
            width,
            placeholder,
            window,
            colors,
            cx,
        ))
}

fn create_table_table_option_label(label: &'static str, colors: UiColors) -> Div {
    div()
        .w(px(84.))
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(label)
}

fn create_table_engine_options() -> Vec<String> {
    [
        "InnoDB",
        "MyISAM",
        "MEMORY",
        "CSV",
        "ARCHIVE",
        "BLACKHOLE",
        "FEDERATED",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn create_table_row_format_options() -> Vec<String> {
    [
        "DEFAULT",
        "DYNAMIC",
        "FIXED",
        "COMPRESSED",
        "REDUNDANT",
        "COMPACT",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}
