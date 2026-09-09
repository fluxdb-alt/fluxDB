fn component_data_table(
    table_state: &Entity<TableState<DataPageTableDelegate>>,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let transparent = cx.theme().transparent;
    let theme = ComponentTheme::global_mut(cx);
    theme.colors.table_hover = transparent;
    theme.colors.table_active = transparent;
    theme.colors.table_active_border = transparent;
    let is_redis_table = table_state.read(cx).delegate().redis_page;

    div()
        .relative()
        .size_full()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .child(
            DataTable::new(&table_state)
                .stripe(false)
                .bordered(false)
                .scrollbar_visible(true, true),
        )
        .when(is_redis_table, |this| {
            let measured_table = table_state.clone();
            this.child(
                canvas(
                    move |bounds, _, cx| {
                        measured_table.update(cx, |table, cx| {
                            if table
                                .delegate_mut()
                                .sync_redis_table_width(redis_table_fit_width(bounds.size.width))
                            {
                                table.refresh(cx);
                            }
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
        })
}

fn data_table_area(
    table_state: &Entity<TableState<DataPageTableDelegate>>,
    loading: bool,
    loading_text: &'static str,
    search_bar: Option<Div>,
    sql_drawer: Option<Div>,
    cell_detail_drawer: Option<Div>,
    cell_detail_drawer_height: f32,
    footer: Option<Div>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let footer_height = if footer.is_some() { 30. } else { 0. };
    let search_height = if search_bar.is_some() { 30. } else { 0. };
    let drawer_height = if sql_drawer.is_some() { 150. } else { 0. };
    let detail_height = if cell_detail_drawer.is_some() {
        clamp_cell_detail_drawer_height(cell_detail_drawer_height)
    } else {
        0.
    };
    div()
        .relative()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .top_0()
                .right_0()
                .bottom(px(
                    footer_height + search_height + drawer_height + detail_height,
                ))
                .left_0()
                .overflow_hidden()
                .child(component_data_table(table_state, cx)),
        )
        .when_some(search_bar, |this, search_bar| {
            this.child(
                search_bar
                    .absolute()
                    .right_0()
                    .bottom(px(footer_height + drawer_height + detail_height))
                    .left_0(),
            )
        })
        .when_some(sql_drawer, |this, sql_drawer| {
            this.child(
                sql_drawer
                    .absolute()
                    .right_0()
                    .bottom(px(footer_height + detail_height))
                    .left_0()
                    .h(px(drawer_height)),
            )
        })
        .when_some(cell_detail_drawer, |this, cell_detail_drawer| {
            this.child(
                cell_detail_drawer
                    .absolute()
                    .right_0()
                    .bottom(px(footer_height))
                    .left_0()
                    .h(px(detail_height)),
            )
        })
        .when_some(footer, |this, footer| {
            this.child(footer.absolute().right_0().bottom_0().left_0())
        })
        .when(loading, |builder| {
            builder.child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .left_0()
                    .bg(if colors.is_dark {
                        hsla(216. / 360., 0.16, 0.10, 0.76)
                    } else {
                        hsla(210. / 360., 0.20, 0.98, 0.76)
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .child(loading_spinner_with_color(18., colors.muted))
                    .child(loading_text)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_scroll_wheel(|_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_mouse_move(|_, _, cx| {
                        cx.stop_propagation();
                    }),
            )
        })
}
