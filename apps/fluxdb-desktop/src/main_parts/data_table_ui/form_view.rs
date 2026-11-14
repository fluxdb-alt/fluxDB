// 数据页表单视图：单行字段纵向排布，外键字段旁直接给跳转入口。
//
// 只改变布局：字段值仍来自当前数据页，编辑通过既有 `EditDataCell` 改动集，
// 保存/还原复用底部工具栏语义，不引入第二套提交流程。

fn data_form_view(
    view: &mut NavicatMain,
    tab_id: TabId,
    object: &ObjectPath,
    page: &DataPage,
    display_row: Option<usize>,
    source_row: Option<usize>,
    filter_summary: &str,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(display_row) = display_row else {
        return div()
            .flex_1()
            .p_6()
            .text_size(ui_text_size(12.))
            .text_color(colors.muted)
            .child(tr("app.data.relations.select_row"));
    };
    let Some(source_row) = source_row else {
        return div()
            .flex_1()
            .p_6()
            .text_size(ui_text_size(12.))
            .text_color(colors.muted)
            .child(tr("app.data.relations.select_row"));
    };
    // `page` 是编辑器原始页，排序后的显示行需换算回源行再取值。
    let Some(row) = page.rows.get(source_row) else {
        return div()
            .flex_1()
            .p_6()
            .text_size(ui_text_size(12.))
            .text_color(colors.muted)
            .child(tr("app.data.form.empty"));
    };
    view.ensure_data_form_inputs(tab_id, &page.columns, window, cx);
    let row_changed = view.data_form_synced_row.get(&tab_id).copied() != Some(source_row);
    if row_changed {
        view.data_form_synced_row.insert(tab_id, source_row);
    }
    view.sync_data_form_inputs(tab_id, &row.values, row_changed, window, cx);
    let targets = view.form_foreign_key_targets(tab_id, object);
    let primary_key = page
        .columns
        .iter()
        .enumerate()
        .find(|(_, column)| column.primary_key)
        .and_then(|(index, _)| row.values.get(index).map(CellValue::display_label))
        .unwrap_or_else(|| (display_row + 1).to_string());
    // 卡片铺满可用宽度，靠字段列数吃掉宽屏空间：单列标签在左，宽屏并排成多列，
    // 既不会出现右侧大片空白，也不会把单个输入框拉成一条长线。
    // 按「扣除连接栏与关系面板后的实际内容宽」判断，靠近阈值时不会突然跳列。
    let mut available_width = tab_workspace_available_width(view, window);
    if view.data_relation_panels.contains(&tab_id) {
        available_width -= view.data_relation_panel_width(tab_id);
    }
    let columns = match available_width {
        width if width >= 2100. => 3,
        width if width >= 1500. => 2,
        _ => 1,
    };
    let per_column = page.columns.len().div_ceil(columns).max(1);
    // 手工配置的候选值与表格单元格共用同一来源；`enum(...)`/布尔类型的候选由类型本身解析。
    let column_choices = view.column_choices_for_tab(tab_id);
    let rows = page
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let input = view.data_form_inputs.get(&(tab_id, index)).cloned();
            let focused = input
                .as_ref()
                .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window));
            let target = targets.iter().find(|target| target.column == column.name);
            let value = row.values.get(index).cloned().unwrap_or(CellValue::Null);
            let choices = column_choices.get(&column.name).cloned().unwrap_or_default();
            let meta = form_data_column_meta(column, choices);
            let kind = data_cell_editor_kind(meta.type_name.as_str());
            // SET 是多值勾选，表单里继续用逗号分隔的自由输入，不给单选菜单。
            let mut options = if kind == DataCellEditorKind::Set {
                Vec::new()
            } else {
                data_cell_choice_options(kind, &meta)
            };
            if !options.is_empty() && column.nullable {
                options.insert(
                    0,
                    ColumnChoice {
                        value: "NULL".to_string(),
                        label: String::new(),
                    },
                );
            }
            data_form_field(
                tab_id,
                object.clone(),
                index,
                column,
                target,
                value,
                input,
                focused,
                options,
                view.data_form_choice_menus.contains(&(tab_id, index)),
                // 分隔线只画在字段之间，每组最后一个字段不加，避免卡片内出现悬空横线。
                index + 1 == page.columns.len()
                    || (index + 1) % per_column == 0 && index + 1 < page.columns.len(),
                colors,
                cx,
            )
        })
        .collect::<Vec<_>>();
    let mut columned = (0..columns)
        .map(|_| Vec::new())
        .collect::<Vec<Vec<Div>>>();
    for (index, row) in rows.into_iter().enumerate() {
        columned[(index / per_column).min(columns - 1)].push(row);
    }
    let body = div()
        .flex()
        .items_start()
        .gap_8()
        .children(columned.into_iter().map(|column| {
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .children(column)
        }));
    let card = div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow(vec![box_shadow(
            px(0.),
            px(2.),
            px(10.),
            px(0.),
            hsla(0., 0., 0., if colors.is_dark { 0.3 } else { 0.06 }),
        )])
        .flex()
        .flex_col()
        .child(data_form_head(
            tab_id,
            &object.name,
            &primary_key,
            display_row,
            page.rows.len(),
            filter_summary,
            colors,
            cx,
        ))
        // 字段区独立滚动：头部（表名、记录主键、上下条）固定在卡片顶部，长表也始终可见。
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .px_4()
                .pb_4()
                .child(body),
        );
    // min_w(0) + overflow_hidden：长字段值不能把表单撑宽，否则右侧关系面板会被挤出窗口。
    div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_hidden()
        // 页面底色比卡片略灰，衬出卡片边界。
        .bg(colors.panel_alt)
        .flex()
        .flex_col()
        .child(div().flex_1().min_h(px(0.)).p_4().flex().child(card))
        // 底部操作条与表格视图底栏同高同底色，切换视图时底边不跳动。
        // 底栏是独立实体：输入框变化只重绘它，不重建整个窗口。
        .child(view.ensure_data_form_footer(tab_id, colors, cx))
}

fn data_form_head(
    tab_id: TabId,
    table: &str,
    primary_key: &str,
    display_row: usize,
    row_count: usize,
    filter_summary: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let sub = if filter_summary.trim().is_empty() {
        tr("app.data.form.all_fields").to_string()
    } else {
        filter_summary.trim().to_string()
    };
    div()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(
                            tr("app.data.form.record_title")
                                .replace("%{table}", table)
                                .replace("%{pk}", primary_key),
                        ),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(format!(
                            "{} · {}",
                            sub,
                            tr("app.data.form.position")
                                .replace("%{index}", &(display_row + 1).to_string())
                                .replace("%{total}", &row_count.to_string())
                        )),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new(("data-form-prev", tab_id.0))
                        .label(tr("app.data.form.previous"))
                        .small()
                        .outline()
                        .disabled(display_row == 0)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.navigate_data_form_row(tab_id, -1, cx);
                        })),
                )
                .child(
                    Button::new(("data-form-next", tab_id.0))
                        .label(tr("app.data.form.next"))
                        .small()
                        .outline()
                        .disabled(display_row + 1 >= row_count)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.navigate_data_form_row(tab_id, 1, cx);
                        })),
                ),
        )
}

#[allow(clippy::too_many_arguments)]
fn data_form_field(
    tab_id: TabId,
    object: ObjectPath,
    index: usize,
    column: &GdbColumn,
    target: Option<&ForeignKeyInfo>,
    value: CellValue,
    input: Option<Entity<InputState>>,
    focused: bool,
    options: Vec<ColumnChoice>,
    menu_open: bool,
    last_in_column: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let type_label = column.type_name.clone().unwrap_or_default();
    let mut type_text = if column.nullable {
        tr("app.data.form.type_nullable").replace("%{type}", &type_label)
    } else {
        type_label
    };
    if let Some(target) = target {
        type_text = format!("{type_text} · {}.{}", target.ref_table, target.ref_column);
    }
    // 标签固定宽度居左，与原型一致：字段名纵向对齐，扫读一列比看块状标题快。
    let label = div()
        .w(px(150.))
        .flex_shrink_0()
        .pt(px(8.))
        .flex()
        .flex_col()
        .gap(px(2.))
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .min_w(px(0.))
                .child(
                    div()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(column.name.clone()),
                )
                // 主键只用一个字母标记，避免徽标框把标签行撑得杂乱。
                .when(column.primary_key, |this| {
                    this.child(
                        div()
                            .text_size(ui_text_size(9.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.warning)
                            .child(tr("app.data.form.pk")),
                    )
                }),
        )
        .child(
            div()
                .text_size(ui_text_size(10.5))
                .text_color(colors.muted)
                .child(type_text),
        );
    let is_null = matches!(value, CellValue::Null);
    // 有候选值时输入框右侧给下拉箭头：点开只提供候选，输入框本身仍可自由输入。
    let has_options = !options.is_empty();
    // 只有带候选菜单的字段才需要把当前值转成文本做勾选比对。
    let current = if has_options {
        data_cell_edit_text(&value)
    } else {
        String::new()
    };
    let mut shell = data_form_input_shell(input.clone(), focused, colors);
    if has_options {
        shell = shell.child(data_form_choice_trigger(tab_id, index, menu_open, colors, cx));
    }
    // 相对定位容器：候选菜单锚在输入框左下角下方展开，浮在后续字段之上。
    let mut field = div()
        .flex_1()
        .min_w(px(0.))
        .relative()
        .child(shell);
    if has_options
        && menu_open
        && let Some(input) = input.clone()
    {
        field = field.child(data_form_choice_menu(
            tab_id, index, input, options, current, colors, cx,
        ));
    }
    let mut line = div().flex().items_center().gap_2().child(field);
    if let Some(target) = target
        && !is_null
    {
        let target_object = ObjectPath {
            connection_id: object.connection_id,
            database: object.database.clone(),
            schema: target.ref_schema.clone().or_else(|| object.schema.clone()),
            name: target.ref_table.clone(),
            kind: ObjectKind::Table,
        };
        let field = target.ref_column.clone();
        line = line.child(
            Button::new(gpui::ElementId::Name(
                format!("data-form-jump-{}-{}", tab_id.0, index).into(),
            ))
                .label(tr("app.data.form.open_related").replace("%{table}", &target.ref_table))
                .small()
                .outline()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_related_table(
                        target_object.clone(),
                        vec![(field.clone(), value.clone())],
                        cx,
                    );
                })),
        );
    }
    // flex_1：输入区吃掉标签列之后的所有宽度，否则 shell 会收缩成一个空方块。
    let mut control = div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap_1()
        .child(line);
    if target.is_some() && is_null {
        control = control.child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.data.form.ref_null")),
        );
    }
    // 一行一字段：左侧标签、右侧输入，字段之间只用细分隔线，不画卡片内卡片。
    div()
        .min_w(px(0.))
        .flex()
        .items_start()
        .gap(px(16.))
        .py(px(10.))
        .when(!last_in_column, |this| {
            this.border_b_1().border_color(colors.border_soft)
        })
        .child(label)
        .child(control)
}

fn data_form_input_shell(
    input: Option<Entity<InputState>>,
    focused: bool,
    colors: UiColors,
) -> Div {
    div()
        .h(px(32.))
        .w_full()
        .min_w(px(0.))
        .rounded_md()
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .px_2()
        .flex()
        .items_center()
        .hover(move |style| style.border_color(colors.text))
        // 焦点描边放在 hover 之后，聚焦时始终可见，点没点进去一目了然。
        .when(focused, |this| this.border_color(colors.info))
        .when_some(input, |this, input| {
            this.child(
                Input::new(&input)
                    .appearance(false)
                    .focus_bordered(false)
                    .w_full()
                    .h_full(),
            )
        })
}

/// 表单/表格共用的列元信息（类型名缺省为 unknown，候选值由调用方补齐）。
fn form_data_column_meta(column: &GdbColumn, choices: Vec<ColumnChoice>) -> DataTableColumnMeta {
    DataTableColumnMeta {
        choices,
        name: column.name.clone(),
        type_name: column
            .type_name
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        comment: column.comment.clone(),
        nullable: column.nullable,
        primary_key: column.primary_key,
    }
}

/// 输入框右侧的下拉箭头：只负责展开/收起候选菜单，不改变输入框的可编辑性。
fn data_form_choice_trigger(
    tab_id: TabId,
    index: usize,
    open: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_none()
        .size(px(22.))
        .rounded(colors.radius * 0.5)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover))
        .child(app_icon(
            AppIcon::ChevronDown,
            12.,
            if open { colors.text } else { colors.muted },
        ))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.open_data_form_choice_menu(tab_id, index, cx);
                cx.stop_propagation();
            }),
        )
}

/// 选中候选后写入输入框并立即提交；输入框仍保持可继续手工输入。
fn data_form_choice_menu(
    tab_id: TabId,
    index: usize,
    input: Entity<InputState>,
    options: Vec<ColumnChoice>,
    current: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let menu_width = data_cell_choice_menu_width(&options);
    let current = current.trim().to_string();
    div()
        .absolute()
        .left(px(0.))
        .bottom(px(0.))
        .size(px(1.))
        .child(
            deferred(
                anchored().anchor(Anchor::TopLeft).child(
                    div()
                        .occlude()
                        .w(px(menu_width))
                        .max_h(px(260.))
                        .overflow_y_scrollbar()
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border)
                        .shadow_lg()
                        .bg(menu_surface_bg(colors))
                        .text_color(colors.text)
                        .p_1()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                            this.close_data_form_choice_menu(tab_id, index, cx);
                        }))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                        .children(options.into_iter().map(|option| {
                            let ColumnChoice { value, label } = option;
                            let checked = if value == "NULL" {
                                current.is_empty()
                            } else {
                                current == value
                            };
                            data_form_choice_item(
                                tab_id,
                                index,
                                input.clone(),
                                value,
                                label,
                                checked,
                                colors,
                                cx,
                            )
                        })),
                ),
            )
            .with_priority(4),
        )
}

#[allow(clippy::too_many_arguments)]
fn data_form_choice_item(
    tab_id: TabId,
    index: usize,
    input: Entity<InputState>,
    value: String,
    label: String,
    checked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let is_null = value == "NULL";
    let value_for_display = value.clone();
    div()
        .h(px(24.))
        .rounded(colors.radius * 0.5)
        .px_2()
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .text_size(ui_text_size(12.))
        .text_color(colors.text)
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .w(px(14.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .when(checked, |this| {
                    this.child(app_icon(AppIcon::Check, 13., colors.muted))
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .child(value_for_display),
        )
        .when(!label.trim().is_empty(), |this| {
            this.child(
                div()
                    .flex_none()
                    .max_w(px(96.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(colors.muted)
                    .child(label),
            )
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                // NULL 对应空文本，由提交流程按可空列转成 NULL。
                let text = if is_null { String::new() } else { value.clone() };
                input.update(cx, |input, cx| {
                    input.set_value(text.clone(), window, cx);
                    input.focus(window, cx);
                });
                this.close_data_form_choice_menu(tab_id, index, cx);
                this.commit_form_input(tab_id, index, text, cx);
                cx.stop_propagation();
            }),
        )
}

fn data_form_foot(
    tab_id: TabId,
    has_pending_changes: bool,
    colors: UiColors,
    cx: &mut Context<DataFormFooter>,
) -> Div {
    // 与 `data_editor_footer` 保持同高（30px）、同底色、同顶部边线，表单/表格切换时底栏不动。
    div()
        .h(px(30.))
        .w_full()
        .flex_none()
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .pl_2()
        .pr_2()
        .flex()
        .items_center()
        .gap_2()
        .child(
            Button::new(("data-form-save", tab_id.0))
                .label(tr("app.data.form.save"))
                .small()
                .primary()
                .disabled(!has_pending_changes)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let parent = this.parent.clone();
                    if let Some(parent) = parent.upgrade() {
                        parent.update(cx, |main, cx| main.save_data_form(tab_id, cx));
                    }
                })),
        )
        .child(
            Button::new(("data-form-revert", tab_id.0))
                .label(tr("app.data.form.revert"))
                .small()
                .outline()
                .disabled(!has_pending_changes)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let parent = this.parent.clone();
                    if let Some(parent) = parent.upgrade() {
                        parent.update(cx, |main, cx| main.revert_data_form(tab_id, cx));
                    }
                })),
        )
        .child(
            div()
                .ml_2()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.data.form.hint")),
        )
}
