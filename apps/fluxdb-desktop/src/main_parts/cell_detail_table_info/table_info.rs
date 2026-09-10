fn table_info_panel(
    tab_id: TabId,
    editor: &DataEditorState,
    page: &DataPage,
    editor_theme: editor_component::EditorTheme,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .relative()
        .w(px(editor.table_info.width))
        .min_w(px(TABLE_INFO_MIN_WIDTH))
        .max_w(px(TABLE_INFO_MAX_WIDTH))
        .h_full()
        .flex_none()
        .border_l_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .child(table_info_resize_handle(
            tab_id,
            editor.table_info.width,
            colors,
            cx,
        ))
        .child(table_info_header(tab_id, editor, page.columns.len(), colors, cx))
        .child(table_info_tabs(
            tab_id,
            editor.table_info.active_tab,
            colors,
            cx,
        ))
        .child(table_info_body(
            tab_id,
            editor,
            page,
            editor_theme,
            window,
            colors,
            cx,
        ))
}

fn table_info_resize_handle(
    tab_id: TabId,
    width: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id("table-info-resize-handle")
        .absolute()
        .left(px(-3.))
        .top(px(0.))
        .bottom(px(0.))
        .w(px(6.))
        .cursor_ew_resize()
        .hover(|style| style.bg(rgb(0x3478f6)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.table_info_resize_start = Some(SidebarResizeStart {
                    x: f32::from(event.position.x),
                    width,
                });
                cx.stop_propagation();
            }),
        )
        .on_drag(TableInfoResizeDrag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_drag_move(cx.listener(
            move |this, event: &DragMoveEvent<TableInfoResizeDrag>, _, cx| {
                if let Some(start) = this.table_info_resize_start {
                    let delta = f32::from(event.event.position.x) - start.x;
                    this.dispatch(
                        AppCommand::SetTableInfoWidth {
                            tab_id,
                            width: start.width - delta,
                        },
                        cx,
                    );
                }
                cx.stop_propagation();
            },
        ))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.table_info_resize_start = None;
                cx.stop_propagation();
            }),
        )
        .bg(colors.app_bg)
}

fn table_info_header(
    tab_id: TabId,
    editor: &DataEditorState,
    column_count: usize,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let table_name = editor.object.name.clone();
    let table_name_for_copy = table_name.clone();

    div()
        .h(px(46.))
        .px_3()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    app_icon_box(AppIcon::Table, 18., 16., colors.muted),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(16.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(table_name),
                )
                .child(
                    div()
                        .flex_none()
                        .px_2()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(format!("{column_count} 字段")),
                )
                .child(
                    Button::new(("copy-table-name", tab_id.0))
                        .ghost()
                        .xsmall()
                        .flex_none()
                        .h(px(22.))
                        .min_w(px(22.))
                        .p_0()
                        .tooltip("复制表名")
                        .child(app_icon(AppIcon::Copy, 15., colors.muted))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                table_name_for_copy.clone(),
                            ));
                            this.show_message("已复制表名", AppMessageKind::Success, cx);
                        })),
                ),
        )
        .child(
            Button::new(("close-table-info", tab_id.0))
                .ghost()
                .xsmall()
                .h(px(24.))
                .w(px(24.))
                .p_0()
                .accessibility_label("关闭表结构面板")
                .tooltip("关闭")
                .child(app_icon(AppIcon::Close, 16., colors.muted))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch(AppCommand::CloseTableInfo(tab_id), cx);
                })),
        )
}

fn table_info_tabs(
    tab_id: TabId,
    active_tab: TableInfoTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let tabs = [
        TableInfoTab::Columns,
        TableInfoTab::Indexes,
        TableInfoTab::ForeignKeys,
        TableInfoTab::Triggers,
        TableInfoTab::Ddl,
    ];
    let selected_index = tabs.iter().position(|tab| *tab == active_tab).unwrap_or(0);
    let view = cx.entity().downgrade();
    let tab_bar = tabs
        .iter()
        .map(|tab| Tab::from(table_info_tab_label(*tab)))
        .fold(
            TabBar::new(("table-info-tabs", tab_id.0))
                .segmented()
                .small()
                .selected_index(selected_index)
                .on_click(move |index, _, cx| {
                    if let Some(tab) = tabs.get(*index).copied() {
                        let _ = view.update(cx, |this, cx| {
                            this.dispatch(AppCommand::SelectTableInfoTab { tab_id, tab }, cx);
                        });
                    }
                }),
            |bar, tab| bar.child(tab),
        );

    div()
        .h(px(38.))
        .px_2()
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .child(tab_bar)
}

fn table_info_tab_label(tab: TableInfoTab) -> &'static str {
    match tab {
        TableInfoTab::Columns => "字段",
        TableInfoTab::Indexes => "索引",
        TableInfoTab::ForeignKeys => "外键",
        TableInfoTab::Triggers => "触发器",
        TableInfoTab::Ddl => "DDL",
    }
}

fn table_info_body(
    tab_id: TabId,
    editor: &DataEditorState,
    page: &DataPage,
    editor_theme: editor_component::EditorTheme,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    match editor.table_info.active_tab {
        TableInfoTab::Columns => table_info_columns(tab_id, editor, page, window, cx),
        TableInfoTab::Indexes => table_info_indexes(
            &editor.table_info.indexes,
            &editor.table_info.search,
            tab_id,
            colors,
            window,
            cx,
        ),
        TableInfoTab::ForeignKeys => table_info_foreign_keys(
            &editor.table_info.foreign_keys,
            &editor.table_info.search,
            tab_id,
            colors,
            window,
            cx,
        ),
        TableInfoTab::Triggers => table_info_triggers(
            &editor.table_info.triggers,
            &editor.table_info.search,
            tab_id,
            colors,
            window,
            cx,
        ),
        TableInfoTab::Ddl => table_info_ddl(tab_id, editor, editor_theme, window, colors, cx),
    }
}

fn table_info_columns(
    tab_id: TabId,
    editor: &DataEditorState,
    page: &DataPage,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let available_width = (editor.table_info.width - 18.).max(240.);
    let name_width = (available_width * 0.36).max(110.);
    let type_width = (available_width * 0.25).max(90.);
    let nullable_width = 58.;
    let comment_width = (available_width - name_width - type_width - nullable_width).max(88.);
    let query = normalized_sidebar_search(&editor.table_info.search);
    let rows = page
        .columns
        .iter()
        .filter(|column| {
            query.is_empty()
                || search_matches_text(&column.name, &query)
                || column
                    .type_name
                    .as_ref()
                    .is_some_and(|value| search_matches_text(value, &query))
                || column
                    .comment
                    .as_ref()
                    .is_some_and(|value| search_matches_text(value, &query))
        })
        .map(|column| {
            let highlighted = editor
                .table_info
                .highlighted_column
                .as_ref()
                .is_some_and(|name| name == &column.name);
            TableInfoTableRow {
                highlighted,
                column_action: Some(column.name.clone()),
                cells: vec![
                    TableInfoTableCell {
                        title: column.name.clone(),
                        badge: column.primary_key.then_some("PK"),
                        mono: false,
                        strong: true,
                    },
                    TableInfoTableCell::mono(
                        column.type_name.as_deref().unwrap_or("unknown").to_string(),
                    ),
                    TableInfoTableCell {
                        title: if column.nullable { "YES" } else { "NO" }.to_string(),
                        badge: None,
                        mono: false,
                        strong: true,
                    },
                    TableInfoTableCell::text(
                        column.comment.as_deref().unwrap_or_default().to_string(),
                    ),
                ],
            }
        })
        .collect::<Vec<_>>();
    table_info_table(
        ("table-info-columns", tab_id.0),
        tab_id,
        vec![
            TableColumn::new("name", "列名")
                .width(px(name_width))
                .min_width(px(96.)),
            TableColumn::new("type", "类型")
                .width(px(type_width))
                .min_width(px(84.)),
            TableColumn::new("nullable", "可空")
                .width(px(nullable_width))
                .resizable(false)
                .movable(false),
            TableColumn::new("comment", "注释")
                .width(px(comment_width))
                .min_width(px(88.)),
        ],
        rows,
        window,
        cx,
    )
}

fn table_info_indexes(
    state: &LoadState<Vec<IndexInfo>>,
    search: &str,
    tab_id: TabId,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    table_info_loaded_list(state, colors, |indexes| {
        let query = normalized_sidebar_search(search);
        let rows = indexes
            .iter()
            .filter(|index| {
                query.is_empty()
                    || search_matches_text(&index.name, &query)
                    || index
                        .columns
                        .iter()
                        .any(|column| search_matches_text(column, &query))
            })
            .map(|index| {
                let badge = if index.is_primary {
                    Some("PK")
                } else if index.is_unique {
                    Some("UNIQUE")
                } else {
                    None
                };
                TableInfoTableRow {
                    highlighted: false,
                    column_action: None,
                    cells: vec![
                        TableInfoTableCell {
                            title: index.name.clone(),
                            badge,
                            mono: false,
                            strong: true,
                        },
                        TableInfoTableCell::mono(index.columns.join(", ")),
                        TableInfoTableCell::mono(
                            index.index_type.clone().unwrap_or_else(|| "-".to_string()),
                        ),
                    ],
                }
            })
            .collect::<Vec<_>>();
        if rows.is_empty() {
            table_info_empty("没有匹配的索引", colors).into_any_element()
        } else {
            table_info_table(
                ("table-info-indexes", tab_id.0),
                tab_id,
                vec![
                    TableColumn::new("name", "索引名").width(px(220.)),
                    TableColumn::new("columns", "字段").width(px(220.)),
                    TableColumn::new("type", "类型")
                        .width(px(110.))
                        .resizable(false)
                        .movable(false),
                ],
                rows,
                window,
                cx,
            )
        }
    })
}

fn table_info_table(
    id: (&'static str, u64),
    tab_id: TabId,
    columns: Vec<TableColumn>,
    rows: Vec<TableInfoTableRow>,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let delegate = TableInfoTableDelegate {
        view: cx.entity().downgrade(),
        tab_id,
        columns,
        rows,
    };
    let table_state = window.use_keyed_state(id, cx, {
        let delegate = delegate.clone();
        |window, cx| {
            TableState::new(delegate, window, cx)
                .sortable(false)
                .row_selectable(false)
                .col_selectable(false)
                .col_movable(false)
                .col_resizable(true)
        }
    });
    table_state.update(cx, |table, cx| {
        *table.delegate_mut() = delegate;
        // row_selectable(false) 在 gpui-component 内部不会拦截点击选中，
        // 表信息表格只做只读展示，这里在刷新时兜底清掉选中态（有选中才清，避免重复通知）。
        if table.selected_row().is_some() || table.selected_col().is_some() {
            table.clear_selection(cx);
        }
        table.refresh(cx);
    });

    div()
        .size_full()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .child(
            DataTable::new(&table_state)
                // 关掉斑马纹：gpui-component 在 `stripe(true)` 时会为填满剩余视口补渲染
                // 「假行」（`rows_count + extra_rows_count`），每行都带下边框 —— 数据集
                // 撑不满面板时下面就会多出一堆空行横线。真实行仍由组件画下边框分隔。
                .stripe(false)
                .bordered(false)
                .small()
                .scrollbar_visible(true, true),
        )
        .into_any_element()
}

fn table_info_foreign_keys(
    state: &LoadState<Vec<ForeignKeyInfo>>,
    search: &str,
    tab_id: TabId,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    table_info_loaded_list(state, colors, |foreign_keys| {
        let query = normalized_sidebar_search(search);
        let rows = foreign_keys
            .iter()
            .filter(|foreign_key| {
                query.is_empty()
                    || search_matches_text(&foreign_key.name, &query)
                    || search_matches_text(&foreign_key.column, &query)
                    || search_matches_text(&foreign_key.ref_table, &query)
                    || search_matches_text(&foreign_key.ref_column, &query)
            })
            .map(|foreign_key| {
                let reference = foreign_key
                    .ref_schema
                    .as_ref()
                    .map(|schema| format!("{schema}."))
                    .unwrap_or_default()
                    + &foreign_key.ref_table
                    + "."
                    + &foreign_key.ref_column;
                TableInfoTableRow {
                    highlighted: false,
                    column_action: None,
                    cells: vec![
                        TableInfoTableCell {
                            title: foreign_key.name.clone(),
                            badge: None,
                            mono: false,
                            strong: true,
                        },
                        TableInfoTableCell::mono(foreign_key.column.clone()),
                        TableInfoTableCell::mono(reference),
                    ],
                }
            })
            .collect::<Vec<_>>();
        if rows.is_empty() {
            table_info_empty("没有匹配的外键", colors).into_any_element()
        } else {
            table_info_table(
                ("table-info-foreign-keys", tab_id.0),
                tab_id,
                vec![
                    TableColumn::new("name", "约束名").width(px(220.)),
                    TableColumn::new("column", "字段").width(px(160.)),
                    TableColumn::new("reference", "引用").width(px(240.)),
                ],
                rows,
                window,
                cx,
            )
        }
    })
}

fn table_info_triggers(
    state: &LoadState<Vec<TriggerInfo>>,
    search: &str,
    tab_id: TabId,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    table_info_loaded_list(state, colors, |triggers| {
        let query = normalized_sidebar_search(search);
        let rows = triggers
            .iter()
            .filter(|trigger| {
                query.is_empty()
                    || search_matches_text(&trigger.name, &query)
                    || search_matches_text(&trigger.event, &query)
                    || search_matches_text(&trigger.timing, &query)
            })
            .map(|trigger| TableInfoTableRow {
                highlighted: false,
                column_action: None,
                cells: vec![
                    TableInfoTableCell {
                        title: trigger.name.clone(),
                        badge: None,
                        mono: false,
                        strong: true,
                    },
                    TableInfoTableCell::mono(trigger.timing.clone()),
                    TableInfoTableCell::mono(trigger.event.clone()),
                ],
            })
            .collect::<Vec<_>>();
        if rows.is_empty() {
            table_info_empty("没有匹配的触发器", colors).into_any_element()
        } else {
            table_info_table(
                ("table-info-triggers", tab_id.0),
                tab_id,
                vec![
                    TableColumn::new("name", "名称").width(px(260.)),
                    TableColumn::new("timing", "时机").width(px(120.)),
                    TableColumn::new("event", "事件").width(px(120.)),
                ],
                rows,
                window,
                cx,
            )
        }
    })
}

fn table_info_ddl(
    tab_id: TabId,
    editor: &DataEditorState,
    editor_theme: editor_component::EditorTheme,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    match &editor.table_info.ddl {
        LoadState::NotLoaded | LoadState::Loading => table_info_loading(colors).into_any_element(),
        LoadState::Failed(error) => {
            table_info_empty(format!("{}：{}", error.title, error.message), colors)
                .into_any_element()
        }
        LoadState::Loaded(ddl) => {
            let ddl_text = ddl.clone();
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(px(34.))
                        .px_3()
                        .border_b_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new(("copy-table-ddl", tab_id.0))
                                .label("复制 DDL")
                                .small()
                                .outline()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        ddl_text.clone(),
                                    ));
                                    this.show_message("已复制 DDL", AppMessageKind::Success, cx);
                                })),
                        )
                        .child(
                            Button::new(("toggle-table-ddl-wrap", tab_id.0))
                                .label(if editor.table_info.ddl_wrap {
                                    "取消换行"
                                } else {
                                    "自动换行"
                                })
                                .small()
                                .outline()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.dispatch(AppCommand::ToggleDdlWrap(tab_id), cx);
                                })),
                        ),
                )
                .child(table_info_ddl_text(
                    tab_id,
                    ddl,
                    editor.table_info.ddl_wrap,
                    editor_theme,
                    window,
                    colors,
                    cx,
                ))
                .into_any_element()
        }
    }
}

fn table_info_loaded_list<T, F>(
    state: &LoadState<Vec<T>>,
    colors: UiColors,
    render: F,
) -> gpui::AnyElement
where
    F: FnOnce(&[T]) -> gpui::AnyElement,
{
    match state {
        LoadState::NotLoaded | LoadState::Loading => table_info_loading(colors).into_any_element(),
        LoadState::Failed(error) => {
            table_info_empty(format!("{}：{}", error.title, error.message), colors)
                .into_any_element()
        }
        LoadState::Loaded(items) if items.is_empty() => {
            table_info_empty("暂无数据", colors).into_any_element()
        }
        LoadState::Loaded(items) => render(items),
    }
}

fn table_info_loading(colors: UiColors) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .child(loading_spinner_with_color(18., colors.muted))
}

fn table_info_empty(text: impl Into<String>, colors: UiColors) -> Div {
    div()
        .flex_1()
        .p_6()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .text_color(colors.muted)
        .child(text.into())
}

/// DDL 只读预览：复用统一的 SQL / DDL 预览编辑器（见 `sql_preview.rs`），
/// 与「设计表」的 SQL / DDL 预览、查询 / Redis 编辑器同一套高亮与配色来源。
fn table_info_ddl_text(
    tab_id: TabId,
    ddl: &str,
    wrap: bool,
    editor_theme: editor_component::EditorTheme,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    // 仅按标签页缓存：换行开关走 apply_settings 就地生效，不必重建编辑器（重建会丢滚动位置）。
    let editor = sql_preview_editor(
        SharedString::from(format!("table-info-ddl-editor-{}", tab_id.0)),
        ddl,
        // 表结构 DDL 由服务端按该连接的方言生成；当前表属性面板不持有连接类型，
        // 按绝大多数场景（MySQL 系）取方言。高亮查询与方言无关，仅折叠/注释标记随方言。
        sql_editor_adapter::SqlDialect::Mysql,
        wrap,
        editor_theme,
        window,
        cx,
    );

    div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .overflow_hidden()
        .child(editor)
        .into_any_element()
}
