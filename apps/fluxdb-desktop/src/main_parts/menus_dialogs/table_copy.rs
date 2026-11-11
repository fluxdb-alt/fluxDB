fn copy_table_mode_rows(
    copy_data: bool,
    running: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(ui_text_size(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(tr("app.table.copy.content")),
        )
        .child(copy_table_mode_row(
            tr("app.table.copy.structure_only"),
            !copy_data,
            false,
            running,
            colors,
            cx,
        ))
        .child(copy_table_mode_row(
            tr("app.table.copy.structure_and_data"),
            copy_data,
            true,
            running,
            colors,
            cx,
        ))
}

fn copy_table_mode_row(
    label: impl Into<SharedString>,
    checked: bool,
    copy_data: bool,
    running: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let label: SharedString = label.into();
    div()
        .debug_selector(move || format!("copy-table-mode:{copy_data}"))
        .h(px(34.))
        .px_3()
        .rounded(colors.radius)
        .border_1()
        .border_color(if checked {
            rgb(0x2563eb)
        } else {
            colors.border_soft
        })
        .bg(if checked {
            colors.tree_selected
        } else {
            colors.panel_bg
        })
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.border_color(colors.border))
        .child(Checkbox::new(("copy-table-mode", copy_data as u64)).checked(checked))
        .child(div().text_size(ui_text_size(13.)).child(label))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if !running {
                    this.set_copy_table_data_mode(copy_data, cx);
                }
                cx.stop_propagation();
            }),
        )
}
