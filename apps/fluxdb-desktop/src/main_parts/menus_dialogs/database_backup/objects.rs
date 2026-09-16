fn database_backup_objects_body(
    form: &BackupForm,
    object_search_input: Entity<InputState>,
    objects_scroll: &VirtualListScrollHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let query = normalized_sidebar_search(&form.object_search);
    // 当前筛选下的表名（排序），供列表与批量操作使用。
    let filtered_tables: Vec<String> = form
        .all_table_names
        .iter()
        .filter(|name| search_matches_text(name, &query))
        .cloned()
        .collect();
    let view = cx.entity();
    let view_tables = form.all_view_names.len();
    // 统计展示：已选表数 + （勾选视图则计入视图数）。
    let selected_tables = form.selected_tables.len();
    let selected_objects = selected_tables + if form.include_views { view_tables } else { 0 };
    let total_objects = form.all_table_names.len() + view_tables;

    let has_tables = !form.all_table_names.is_empty();
    // 表列表虚拟化：只渲染视口附近的行，表多时避免每帧重建整棵行 DOM（fps 掉到 20 的根因）。
    // 行高固定 32px，item_sizes 每行给 size(0, 32)；渲染期从实体内读取 selected_tables 判勾选态。
    let list_scroll = if filtered_tables.is_empty() {
        // 空状态：搜索无结果或本无表，用基础布局 + 说明文字。
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .child(app_icon(AppIcon::Search, 22., colors.muted))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(colors.muted)
                    .child(if has_tables {
                        "未找到匹配的表"
                    } else {
                        "暂无可备份对象"
                    }),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .when(has_tables, |d| d.child("请尝试其他关键词")),
            )
            .into_any_element()
    } else {
        let scroll = objects_scroll.clone();
        let table_names: Rc<Vec<String>> = Rc::new(filtered_tables.clone());
        let item_sizes: Rc<Vec<Size<Pixels>>> =
            Rc::new(vec![size(px(0.), px(32.)); filtered_tables.len()]);
        let list_view = cx.entity();
        v_virtual_list(
            list_view,
            "backup-objects-table-vlist",
            item_sizes,
            move |this, range, _window, cx| {
                range
                    .map(|ix| {
                        let name = &table_names[ix];
                        let checked = this
                            .pending_backup_modal
                            .as_ref()
                            .map(|f| f.selected_tables.contains(name))
                            .unwrap_or(false);
                        database_backup_table_row(name, checked, colors, cx)
                    })
                    .collect()
            },
        )
        .track_scroll(&scroll)
        .flex_1()
        .min_h(px(0.))
        .pt_1()
        .into_any_element()
    };

    // 搜索与操作栏保持紧凑，剩余高度全部分配给表列表。
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .gap_2()
        // 搜索框：左侧搜索图标、右侧输入非空时显示清除按钮。
        .child(
            div()
                .id("backup-object-search")
                .track_focus(&object_search_input.read(cx).focus_handle(cx))
                .flex_none()
                .h(px(34.))
                .px_2()
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .hover(move |s| s.border_color(colors.muted))
                .focus(move |s| {
                    s.border_color(if colors.is_dark {
                        rgb(0x8ab4ff)
                    } else {
                        rgb(0x111111)
                    })
                })
                .bg(colors.input_bg)
                .flex()
                .items_center()
                .gap_2()
                .child(app_icon(AppIcon::Search, 15., colors.muted))
                .child(
                    div().flex_1().min_w(px(0.)).child(
                        Input::new(&object_search_input)
                            .appearance(false)
                            .focus_bordered(false)
                            .cleanable(true)
                            .w_full()
                            .h_full()
                            .text_size(px(13.)),
                    ),
                ),
        )
        // 批量操作工具条：全选复选框 + 结果数量 + 清空按钮。
        .child(
            div()
                .h(px(28.))
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Checkbox::new("backup-select-all")
                        .label("全选表")
                        .cursor_pointer()
                        .disabled(filtered_tables.is_empty())
                        .checked(
                            !filtered_tables.is_empty()
                                && filtered_tables
                                    .iter()
                                    .all(|n| form.selected_tables.contains(n)),
                        )
                        .on_click({
                            let view = view.clone();
                            // 全选/取消全选：当前筛选中存在未选中的表则全选，否则全部取消。
                            let select_all = filtered_tables
                                .iter()
                                .any(|n| !form.selected_tables.contains(n));
                            move |_, _, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.set_backup_filtered_tables(select_all, cx);
                                });
                            }
                        }),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(format!(
                            "{} 项 · 已选 {selected_objects}/{total_objects}",
                            filtered_tables.len()
                        )),
                )
                .child(div().flex_1())
                .child(
                    Button::new("backup-clear-btn")
                        .label("清空")
                        .small()
                        .ghost()
                        .rounded_md()
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_backup_filtered_tables(false, cx);
                            cx.stop_propagation();
                        })),
                ),
        )
        // 表列表：带边框、圆角、背景色的滚动容器。虚拟化列表的滚动条挂在此容器上。
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .flex()
                .flex_col()
                .overflow_hidden()
                .vertical_scrollbar(&objects_scroll.clone())
                .child(list_scroll),
        )
        // 视图分组：数据模型为单一开关（include_views），非逐视图勾选，故只渲染一行。
        .when(view_tables > 0, |this| {
            this.child(
                div().h(px(28.)).flex_none().flex().items_center().child(
                    Checkbox::new("backup-include-views")
                        .label(format!("包含视图（{view_tables}）"))
                        .checked(form.include_views)
                        .cursor_pointer()
                        .on_click(move |checked, _, cx| {
                            view.update(cx, |this, cx| this.set_backup_include_views(*checked, cx));
                            cx.stop_propagation();
                        }),
                ),
            )
        })
        // 底部提示：次级文字 + 图标，说明对象选择仅对逻辑备份生效。
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(app_icon(AppIcon::List, 13., colors.muted))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child("对象选择仅对「逻辑备份」生效；原生备份导出整库。"),
                ),
        )
}

/// 对象选择页签：单张表的整行行。整行可点击切换选择；Checkbox 为受控展示（选中态），
/// 其 on_click 停止冒泡，避免与整行点击双重触发。
fn database_backup_table_row(
    name: &str,
    checked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui_component::list::ListItem {
    let view = cx.entity();
    let name_string = name.to_string();
    gpui_component::list::ListItem::new(format!("backup-obj-row-{name_string}"))
        .h(px(32.))
        .w_full()
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .rounded(colors.radius)
        .cursor_pointer()
        .selected(checked)
        // 整行用 on_click（抬起阶段）切换：与 Checkbox 同为点击阶段，勾选框 on_click
        // stop_propagation 后可避免「按下一行 + 抬起勾选框」造成的双重切换。
        .on_click({
            let toggle_name = name_string.clone();
            let view = view.clone();
            move |_, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.toggle_backup_table(&toggle_name, cx);
                });
                cx.stop_propagation();
            }
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Checkbox::new(format!("backup-table-{name}"))
                        .checked(checked)
                        .accessibility_label(name_string.clone())
                        .cursor_pointer()
                        .on_click({
                            let toggle_name = name_string.clone();
                            move |_, _, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.toggle_backup_table(&toggle_name, cx);
                                });
                                cx.stop_propagation();
                            }
                        }),
                )
                .child(
                    div().flex_1().min_w(px(0.)).child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(colors.text)
                            .child(name_string),
                    ),
                ),
        )
}
