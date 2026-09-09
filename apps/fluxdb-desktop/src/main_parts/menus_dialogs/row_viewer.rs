fn data_row_viewer_modal(
    viewer: DataRowViewer,
    state: &AppState,
    search_input: Entity<InputState>,
    search: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let view = cx.entity();
    let Some((table_name, page_offset, fields)) = data_row_viewer_snapshot(&viewer, state) else {
        return div();
    };
    let row_number = page_offset + viewer.source_row as u64 + 1;
    let title = format!("第 {row_number} 行详情");
    let subtitle = format!("{} · {} 列", table_name, fields.len());

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
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.close_data_row_viewer(cx);
                cx.stop_propagation();
            }),
        )
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .w(px(760.))
                .h(px(680.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(menu_surface_bg(colors))
                .occlude()
                .overflow_hidden()
                .text_color(colors.text)
                .flex()
                .flex_col()
                .key_context("DataRowViewer")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.close_data_row_viewer(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(data_row_viewer_header(
                    title,
                    subtitle,
                    view.clone(),
                    colors,
                ))
                .child(data_row_detail_viewer_body(
                    viewer.tab_id,
                    viewer.query_result_page_index,
                    viewer.source_row,
                    fields,
                    search_input,
                    search,
                    colors,
                    cx,
                )),
        )
}

fn data_row_viewer_snapshot(
    viewer: &DataRowViewer,
    state: &AppState,
) -> Option<(String, u64, Vec<RowFieldSnapshot>)> {
    state.tabs.iter().find_map(|tab| {
        if tab.id != viewer.tab_id {
            return None;
        }
        match &tab.kind {
            TabKind::DataEditor(editor) => {
                let page = editor.page.as_ref()?;
                let fields = row_fields_for_page(page, viewer.source_row)?;
                Some((editor.object.name.clone(), page.offset, fields))
            }
            TabKind::QueryEditor(editor) => {
                let page_index = viewer.query_result_page_index.or(editor.active_result_editor)?;
                let editable = editor.result_editors.get(&page_index);
                let page = editable
                    .and_then(|editor| editor.page.as_ref())
                    .or_else(|| editor.results.get(page_index))?;
                let fields = row_fields_for_page(page, viewer.source_row)?;
                let name = editable
                    .map(|editor| editor.object.name.clone())
                    .unwrap_or_else(|| format!("查询结果 {}", page_index + 1));
                Some((name, page.offset, fields))
            }
            _ => None,
        }
    })
}

fn data_row_viewer_header(
    title: String,
    subtitle: String,
    view: Entity<NavicatMain>,
    colors: UiColors,
) -> Div {
    div()
        .h(px(58.))
        .flex_none()
        .px_4()
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(subtitle),
                ),
        )
        .child(
            cell_detail_icon_button(AppIcon::Close, "关闭", colors).on_mouse_down(
                MouseButton::Left,
                move |_, _, cx| {
                    view.update(cx, |this, cx| this.close_data_row_viewer(cx));
                    cx.stop_propagation();
                },
            ),
        )
}

fn data_row_detail_viewer_body(
    tab_id: TabId,
    query_result_page_index: Option<usize>,
    source_row: usize,
    fields: Vec<RowFieldSnapshot>,
    search_input: Entity<InputState>,
    search: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let needle = search.trim().to_ascii_lowercase();
    let visible_fields = fields
        .into_iter()
        .filter(|field| {
            if needle.is_empty() {
                return true;
            }
            field.name.to_ascii_lowercase().contains(&needle)
                || field.type_name.to_ascii_lowercase().contains(&needle)
                || field
                    .comment
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains(&needle)
                || cell_value_label(&field.value)
                    .to_ascii_lowercase()
                    .contains(&needle)
        })
        .collect::<Vec<_>>();

    let mut list = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .p_3()
        .flex()
        .flex_col()
        .gap_2();
    for field in visible_fields {
        list = list.child(data_row_detail_field(field, colors, cx));
    }

    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(46.))
                .flex_none()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(colors.border_soft)
                .child(
                    div()
                        .h_full()
                        .rounded(colors.radius_lg)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.input_bg)
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child(app_icon(AppIcon::Search, 14., colors.muted))
                        .child(
                            Input::new(&search_input)
                                .appearance(false)
                                .focus_bordered(false)
                                .w_full()
                                .h_full()
                                .text_size(px(13.)),
                        ),
                ),
        )
        .child(list)
        .child(
            div()
                .h(px(44.))
                .flex_none()
                .px_4()
                .border_t_1()
                .border_color(colors.border)
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .child(
                    cell_detail_action_button("复制 JSON", colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.copy_data_row_text(
                                tab_id,
                                query_result_page_index,
                                source_row,
                                DataRowCopyKind::Json,
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    ),
                )
                .child(cell_detail_action_button("复制 TSV", colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.copy_data_row_text(
                            tab_id,
                            query_result_page_index,
                            source_row,
                            DataRowCopyKind::Tsv,
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                )),
        )
}

fn data_row_detail_field(
    field: RowFieldSnapshot,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let type_color = data_type_color(&field.type_name).unwrap_or(colors.text);
    let value_text = cell_value_label(&field.value);
    let copy_value = value_text.clone();
    let length = cell_value_length(&field.value);

    div()
        .min_h(px(72.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border_soft)
        .bg(if colors.is_dark {
            rgb(0x171b21)
        } else {
            rgb(0xffffff)
        })
        .px_3()
        .py_2()
        .flex()
        .items_start()
        .gap_3()
        .child(
            div()
                .w(px(38.))
                .flex_none()
                .pt_1()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(field.index.to_string()),
        )
        .child(
            div()
                .w(px(190.))
                .flex_none()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(field.name),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(11.))
                        .child(div().text_color(type_color).child(field.type_name))
                        .when(field.primary_key, |this| {
                            this.child(
                                div()
                                    .rounded(colors.radius * 0.5)
                                    .px_1()
                                    .bg(if colors.is_dark {
                                        rgb(0x18365f)
                                    } else {
                                        rgb(0xe7f0ff)
                                    })
                                    .text_color(if colors.is_dark {
                                        rgb(0xa9c9ff)
                                    } else {
                                        rgb(0x1757bd)
                                    })
                                    .child("主键"),
                            )
                        }),
                )
                .when_some(field.comment, |this, comment| {
                    this.child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(comment),
                    )
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child("值")
                        .child(format!("{length} 字符")),
                )
                .child(
                    div()
                        .min_h(px(30.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border_soft)
                        .bg(colors.input_bg)
                        .px_2()
                        .py_1()
                        .text_size(px(12.))
                        .text_color(colors.text)
                        .overflow_hidden()
                        .child(value_text),
                ),
        )
        .child(
            cell_detail_icon_button(AppIcon::Copy, "复制值", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_value.clone()));
                    this.show_message("已复制值", AppMessageKind::Success, cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

fn data_row_copy_submenu(
    menu: DataRowContextMenu,
    window: &Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selected_rows = (menu.selection_row_count > 1).then_some(menu.selection_row_count);
    let json_label = row_copy_label(selected_rows, "复制行 (JSON)", "复制选中 {count} 行 (JSON)");
    let insert_label = row_copy_label(
        selected_rows,
        "复制为 INSERT 语句",
        "复制选中 {count} 行为 INSERT 语句",
    );
    let insert_without_pk_label = row_copy_label(
        selected_rows,
        "复制为 INSERT 语句（不含主键）",
        "复制选中 {count} 行为 INSERT 语句（不含主键）",
    );
    let update_label = row_copy_label(
        selected_rows,
        "复制为 UPDATE 语句",
        "复制选中 {count} 行为 UPDATE 语句",
    );
    let tsv_label = row_copy_label(selected_rows, "复制全部 (TSV)", "复制选中 {count} 行 (TSV)");
    let width = if menu.row_object_available {
        data_row_copy_submenu_width([
            json_label.as_str(),
            insert_label.as_str(),
            insert_without_pk_label.as_str(),
            update_label.as_str(),
            tsv_label.as_str(),
        ])
    } else {
        data_row_copy_submenu_width([json_label.as_str(), tsv_label.as_str()])
    };
    let height = if menu.row_object_available { 5. } else { 2. } * 26. + 8.;

    data_row_submenu_shell(
        data_row_submenu_top(&menu, 34., height, window),
        data_row_submenu_left(&menu, width, window),
        px(width),
        colors,
    )
        .child(data_row_copy_item(
            json_label,
            DataRowCopyKind::Json,
            menu.clone(),
            colors,
            cx,
        ))
        .when(menu.row_object_available, |this| {
            this.child(data_row_copy_item(
                insert_label,
                DataRowCopyKind::Insert,
                menu.clone(),
                colors,
                cx,
            ))
            .child(data_row_copy_item(
                insert_without_pk_label,
                DataRowCopyKind::InsertWithoutPrimaryKey,
                menu.clone(),
                colors,
                cx,
            ))
            .child(data_row_copy_item(
                update_label,
                DataRowCopyKind::Update,
                menu.clone(),
                colors,
                cx,
            ))
        })
        .child(data_row_copy_item(
            tsv_label,
            DataRowCopyKind::Tsv,
            menu,
            colors,
            cx,
        ))
}

fn data_row_export_submenu(
    menu: DataRowContextMenu,
    window: &Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selected_rows = (menu.selection_row_count > 1).then_some(menu.selection_row_count);
    let csv_label = row_export_label(selected_rows, "导出当前行为 CSV", "导出选中 {count} 行为 CSV");
    let json_label =
        row_export_label(selected_rows, "导出当前行为 JSON", "导出选中 {count} 行为 JSON");
    let markdown_label = row_export_label(
        selected_rows,
        "导出当前行为 Markdown",
        "导出选中 {count} 行为 Markdown",
    );
    let insert_label = row_export_label(
        selected_rows,
        "导出当前行为 SQL INSERT",
        "导出选中 {count} 行为 SQL INSERT",
    );
    let width = if menu.row_object_available {
        data_row_copy_submenu_width([
            csv_label.as_str(),
            json_label.as_str(),
            markdown_label.as_str(),
            insert_label.as_str(),
        ])
    } else {
        data_row_copy_submenu_width([
            csv_label.as_str(),
            json_label.as_str(),
            markdown_label.as_str(),
        ])
    };
    let desired_top = if menu.rows_editable { 148. } else { 68. };
    let height = if menu.row_object_available { 4. } else { 3. } * 26. + 8.;

    data_row_submenu_shell(
        data_row_submenu_top(&menu, desired_top, height, window),
        data_row_submenu_left(&menu, width, window),
        px(width),
        colors,
    )
        .child(data_row_export_item(
            csv_label,
            DataRowExportFormat::Csv,
            menu.clone(),
            colors,
            cx,
        ))
        .child(data_row_export_item(
            json_label,
            DataRowExportFormat::Json,
            menu.clone(),
            colors,
            cx,
        ))
        .child(data_row_export_item(
            markdown_label,
            DataRowExportFormat::Markdown,
            menu.clone(),
            colors,
            cx,
        ))
        .when(menu.row_object_available, |this| {
            this.child(data_row_export_item(
                insert_label,
                DataRowExportFormat::SqlInsert,
                menu,
                colors,
                cx,
            ))
        })
}

fn data_row_submenu_top(
    menu: &DataRowContextMenu,
    desired_top: f32,
    height: f32,
    window: &Window,
) -> Pixels {
    let margin = 8.;
    let viewport_height = f32::from(window.viewport_size().height);
    let menu_y = f32::from(menu.position.y);
    let min_top = margin - menu_y;
    let max_top = viewport_height - menu_y - height - margin;
    px(desired_top.clamp(min_top.min(max_top), max_top.max(min_top)))
}

fn data_row_submenu_left(menu: &DataRowContextMenu, width: f32, window: &Window) -> Pixels {
    let margin = 8.;
    let viewport_width = f32::from(window.viewport_size().width);
    let menu_x = f32::from(menu.position.x);
    let right_left = 194.;
    if menu_x + right_left + width + margin <= viewport_width {
        px(right_left)
    } else {
        px(-width + 2.)
    }
}

fn data_row_submenu_shell(top: Pixels, left: Pixels, width: Pixels, colors: UiColors) -> Div {
    div()
        .absolute()
        .left(left)
        .top(top)
        .w(width)
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(menu_surface_bg(colors))
        .occlude()
        .shadow(vec![box_shadow(
            px(0.),
            px(14.),
            px(30.),
            px(0.),
            hsla(0., 0., 0., 0.18),
        )])
        .p_1()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(colors.text)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
}

fn data_row_menu_item(
    label: &'static str,
    icon: AppIcon,
    tab_id: TabId,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    data_cell_menu_item(label, icon, true, colors)
        .font_weight(gpui::FontWeight::BOLD)
        .on_mouse_move(cx.listener(move |this, _, _, cx| {
            this.set_data_row_context_submenu(tab_id, None, cx);
            cx.stop_propagation();
        }))
}

fn data_row_copy_item(
    label: impl Into<String>,
    kind: DataRowCopyKind,
    menu: DataRowContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    data_cell_menu_item_text(label, AppIcon::Copy, true, colors)
        .font_weight(gpui::FontWeight::BOLD)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.copy_data_row_text(
                    menu.tab_id,
                    menu.query_result_page_index,
                    menu.source_row,
                    kind,
                    cx,
                );
                cx.stop_propagation();
            }),
        )
}

fn row_copy_label(selected_rows: Option<usize>, single: &str, multiple: &str) -> String {
    selected_rows
        .map(|count| multiple.replace("{count}", count.to_string().as_str()))
        .unwrap_or_else(|| single.to_string())
}

fn data_row_export_item(
    label: impl Into<String>,
    format: DataRowExportFormat,
    menu: DataRowContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    data_cell_menu_item_text(label, AppIcon::Save, true, colors)
        .font_weight(gpui::FontWeight::BOLD)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.export_data_rows(
                    menu.tab_id,
                    menu.query_result_page_index,
                    menu.source_row,
                    format,
                    cx,
                );
                cx.stop_propagation();
            }),
        )
}

fn row_export_label(selected_rows: Option<usize>, single: &str, multiple: &str) -> String {
    selected_rows
        .map(|count| multiple.replace("{count}", count.to_string().as_str()))
        .unwrap_or_else(|| single.to_string())
}

fn data_row_copy_submenu_width<const N: usize>(labels: [&str; N]) -> f32 {
    let longest = labels
        .iter()
        .map(|label| label.chars().map(data_row_copy_label_width_unit).sum::<f32>())
        .fold(0., f32::max);
    (longest * 14. + 56.).clamp(278., 440.)
}

fn data_row_copy_label_width_unit(ch: char) -> f32 {
    if ch.is_ascii() { 0.56 } else { 1.0 }
}

fn data_row_menu_submenu_item(
    label: &'static str,
    icon: AppIcon,
    tab_id: TabId,
    submenu: DataRowContextSubmenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id((
            "data-row-submenu-item",
            tab_id.0 as usize + submenu as usize,
        ))
        .h(px(26.))
        .rounded(colors.radius)
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(colors.text)
        .hover(move |style| style.bg(colors.hover))
        .on_hover(cx.listener(move |this, hovered, _, cx| {
            if *hovered {
                this.set_data_row_context_submenu(tab_id, Some(submenu), cx);
            }
        }))
        .child(app_icon(icon, 14., colors.muted))
        .child(div().flex_1().child(label))
        .child(app_icon(AppIcon::ChevronRight, 14., colors.muted))
}
