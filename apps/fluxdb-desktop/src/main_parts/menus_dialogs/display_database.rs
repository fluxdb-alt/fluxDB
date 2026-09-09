fn display_database_modal(
    connection_id: ConnectionId,
    state: &AppState,
    selected: &BTreeSet<String>,
    search: &str,
    search_input: Entity<InputState>,
    show_system: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let view = cx.entity();
    let connection = state
        .connections
        .iter()
        .find(|connection| connection.config.id == connection_id);
    let connection_name = connection
        .map(|connection| connection.config.name.clone())
        .unwrap_or_else(|| "连接".to_string());
    let visible_filter_enabled = connection
        .and_then(|connection| configured_visible_databases(&connection.config.options))
        .is_some();
    let databases = connection
        .map(all_connection_database_names)
        .unwrap_or_default();
    let search_lower = search.trim().to_ascii_lowercase();
    let shown_databases = databases
        .iter()
        .filter(|database| show_system || !is_system_database(database))
        .filter(|database| {
            search_lower.is_empty() || database.to_ascii_lowercase().contains(&search_lower)
        })
        .cloned()
        .collect::<Vec<_>>();
    let selected_count = shown_databases
        .iter()
        .filter(|database| selected.contains(*database))
        .count();

    let mut database_list = div().flex().flex_col().gap_1().p_3();
    for database in &shown_databases {
        database_list = database_list.child(display_database_row(
            database.clone(),
            selected.contains(database),
            colors,
            cx,
        ));
    }

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.08, 0.62)
        } else {
            opaque_grey(0.6, 0.36)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Right, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(
            div()
                .w(px(620.))
                .h(px(720.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(menu_surface_bg(colors))
                .occlude()
                .flex()
                .flex_col()
                .overflow_hidden()
                .text_color(colors.text)
                .key_context("DisplayDatabaseModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_display_database_modal(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_5()
                        .pb_3()
                        .flex()
                        .items_start()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(px(22.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child("显示数据库"),
                                )
                                .child(div().text_size(px(14.)).text_color(colors.muted).child(
                                    format!(
                                        "选择「{}」下要在侧边栏显示的数据库。",
                                        connection_name
                                    ),
                                )),
                        )
                        .child(
                            Button::new("display-database-close")
                                .label("×")
                                .ghost()
                                .w(px(32.))
                                .h(px(32.))
                                .text_size(px(22.))
                                .on_click({
                                    let view = view.clone();
                                    move |_, _, cx| {
                                        view.update(cx, |this, cx| {
                                            this.cancel_display_database_modal(cx);
                                        });
                                        cx.stop_propagation();
                                    }
                                }),
                        ),
                )
                .child(
                    div()
                        .mx_5()
                        .h(px(44.))
                        .rounded(colors.radius_lg)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.input_bg)
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(15.))
                        .text_color(colors.muted)
                        .child(app_icon(AppIcon::Search, 16., colors.muted))
                        .child(
                            Input::new(&search_input)
                                .appearance(false)
                                .focus_bordered(false)
                                .w_full()
                                .h_full()
                                .text_size(px(15.)),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .py_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_size(px(14.))
                        .text_color(colors.muted)
                        .child(format!("已选择 {selected_count}/{}", shown_databases.len()))
                        .child(
                            div()
                                .flex()
                                .gap_4()
                                .child(display_database_link(
                                    "全选",
                                    true,
                                    cx.listener(|this, _, _, cx| {
                                        this.select_all_display_databases(cx);
                                        cx.stop_propagation();
                                    }),
                                ))
                                .child(display_database_link(
                                    "清空",
                                    true,
                                    cx.listener(|this, _, _, cx| {
                                        this.clear_display_databases(cx);
                                        cx.stop_propagation();
                                    }),
                                ))
                                .child(display_database_link(
                                    "显示全部",
                                    visible_filter_enabled,
                                    cx.listener(|this, _, _, cx| {
                                        this.show_all_display_databases(cx);
                                        cx.stop_propagation();
                                    }),
                                )),
                        ),
                )
                .child(div().px_5().pb_3().child(
                    display_database_checkbox("显示系统库", show_system, colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.toggle_display_system_databases(cx);
                            cx.stop_propagation();
                        }),
                    ),
                ))
                .child(
                    div()
                        .id("display-database-list-scroll")
                        .mx_5()
                        .flex_1()
                        .min_h(px(0.))
                        .rounded(colors.radius_lg)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.input_bg)
                        .overflow_scroll()
                        .scrollbar_width(px(8.))
                        .child(database_list),
                )
                .child(
                    div()
                        .h(px(72.))
                        .mt_5()
                        .px_5()
                        .border_t_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_3()
                        .child(
                            Button::new("display-database-cancel")
                                .label("取消")
                                .on_click({
                                    let view = view.clone();
                                    move |_, _, cx| {
                                        view.update(cx, |this, cx| {
                                            this.cancel_display_database_modal(cx);
                                        });
                                        cx.stop_propagation();
                                    }
                                }),
                        )
                        .child(
                            Button::new("display-database-save")
                                .label("保存")
                                .primary()
                                .on_click({
                                    let view = view.clone();
                                    move |_, _, cx| {
                                        view.update(cx, |this, cx| {
                                            this.save_display_database_modal(cx);
                                        });
                                        cx.stop_propagation();
                                    }
                                }),
                        ),
                ),
        )
}

fn display_database_link(
    label: &'static str,
    enabled: bool,
    listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .text_color(if enabled {
            rgb(0x8aa2c7)
        } else {
            rgb(0x626a76)
        })
        .when(enabled, |this| {
            this.hover(|style| style.text_color(rgb(0xffffff)))
                .on_mouse_down(MouseButton::Left, listener)
        })
        .child(label)
}

fn display_database_row(
    database: String,
    selected: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    display_database_checkbox(database.clone(), selected, colors).on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _, _, cx| {
            this.toggle_display_database(database.clone(), cx);
            cx.stop_propagation();
        }),
    )
}

fn display_database_checkbox(label: impl Into<String>, selected: bool, colors: UiColors) -> Div {
    div()
        .h(px(34.))
        .rounded(colors.radius)
        .px_2()
        .flex()
        .items_center()
        .gap_3()
        .text_size(px(14.))
        .text_color(colors.text)
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .size(px(18.))
                .rounded(colors.radius * 0.5)
                .border_1()
                .border_color(if selected { colors.text } else { colors.border })
                .bg(if selected {
                    colors.panel_alt
                } else {
                    colors.input_bg
                })
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(14.))
                .child(if selected { "✓" } else { "" }),
        )
        .child(
            div()
                .overflow_hidden()
                .whitespace_nowrap()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(label.into()),
        )
}

