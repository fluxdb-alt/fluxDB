// 备份配置弹框渲染（Navicat 风格四页签：常规 / 对象选择 / 高级 / 消息日志）。
// 纯 UI + 页签/选项的 setter；执行逻辑在 database_backup.rs。

impl NavicatMain {
    fn set_backup_tab(&mut self, tab: BackupTab, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_backup_modal {
            form.tab = tab;
            cx.notify();
        }
    }

    fn set_backup_include_views(&mut self, value: bool, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_backup_modal {
            form.include_views = value;
            cx.notify();
        }
    }

    fn set_backup_lock_tables(&mut self, value: bool, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_backup_modal {
            form.lock_tables = value;
            cx.notify();
        }
    }

    fn set_backup_single_transaction(&mut self, value: bool, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_backup_modal {
            form.single_transaction = value;
            cx.notify();
        }
    }

    fn set_backup_include_routines(&mut self, value: bool, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_backup_modal {
            form.include_routines = value;
            cx.notify();
        }
    }

    fn set_backup_include_schema(&mut self, value: bool, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_backup_modal {
            form.include_schema = value;
            cx.notify();
        }
    }

    fn set_backup_include_data(&mut self, value: bool, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_backup_modal {
            form.include_data = value;
            cx.notify();
        }
    }

    /// 对象选择：勾选/取消勾选一张表（空集合 = 全选，勾选后精确到表）。
    fn toggle_backup_table(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_backup_modal {
            if !form.selected_tables.insert(name.to_string()) {
                form.selected_tables.remove(name);
            }
            cx.notify();
        }
    }

    /// 对象选择：全选/清空「当前筛选结果」里的表（不触碰未显示的对象，视图由 include_views 独立控制）。
    fn set_backup_filtered_tables(&mut self, selected: bool, cx: &mut Context<Self>) {
        let Some(form) = &mut self.pending_backup_modal else {
            return;
        };
        let query = normalized_sidebar_search(&form.object_search);
        for name in form
            .all_table_names
            .iter()
            .filter(|name| search_matches_text(name, &query))
        {
            if selected {
                form.selected_tables.insert(name.clone());
            } else {
                form.selected_tables.remove(name);
            }
        }
        cx.notify();
    }
}

fn database_backup_modal(
    form: BackupForm,
    file_name_input: Entity<InputState>,
    note_input: Entity<InputState>,
    object_search_input: Entity<InputState>,
    objects_scroll: &VirtualListScrollHandle,
    tasks: &[BackupTaskState],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    // 至少勾选一个对象（任一表或包含视图）才允许「开始备份」；备份目录统一取自设置。
    let can_start = !form.selected_tables.is_empty() || form.include_views;
    let panel = database_backup_modal_panel(colors, cx)
        .child(database_backup_modal_header(colors, cx))
        .child(database_backup_tabs(form.tab, colors, cx))
        .child(database_backup_modal_body(
            &form,
            file_name_input,
            note_input,
            object_search_input,
            objects_scroll,
            colors,
            cx,
        ))
        .child(database_backup_modal_footer(
            can_start,
            None,
            tasks,
            colors,
            cx,
        ));
    database_backup_modal_shell(panel, colors, cx)
}

/// 任务日志弹框：任务已提交后（form 已关闭）展示执行进度、跳过项与取消/清除操作。
fn database_backup_log_modal(
    task: BackupTaskState,
    all_tasks: &[BackupTaskState],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let panel = database_backup_modal_panel(colors, cx)
        .child(database_backup_modal_header(colors, cx))
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .p_5()
                .flex()
                .flex_col()
                .gap_3()
                .child(database_backup_task_card(&task, colors, cx)),
        )
        .child(database_backup_modal_footer(
            false,
            Some(task.id),
            all_tasks,
            colors,
            cx,
        ));
    database_backup_modal_shell(panel, colors, cx)
}

fn database_backup_modal_shell(panel: Div, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.58)
        } else {
            opaque_grey(0.75, 0.28)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.cancel_backup_modal(cx);
            cx.stop_propagation();
        }))
        .child(panel)
}

fn database_backup_modal_panel(colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .w(px(600.))
        .max_w(px(600.))
        .h(px(480.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(menu_surface_bg(colors))
        .shadow(vec![box_shadow(
            px(0.),
            px(18.),
            px(42.),
            px(0.),
            hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
        )])
        .flex()
        .flex_col()
        .overflow_hidden()
        .text_size(px(13.))
        .text_color(colors.text)
        .key_context("DatabaseBackupModal")
        .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
            this.cancel_backup_modal(cx);
            cx.stop_propagation();
        }))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn database_backup_modal_header(colors: UiColors, cx: &mut Context<NavicatMain>) -> impl IntoElement {
    div()
        .h(px(56.))
        .flex_none()
        .px_5()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(app_icon(AppIcon::Save, 18., colors.text))
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("数据库备份"),
                ),
        )
        .child(
            div()
                .size(px(30.))
                .rounded(colors.radius)
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(colors.hover))
                .child(app_icon(AppIcon::Close, 16., colors.muted))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.cancel_backup_modal(cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

/// 四页签（Navicat 风格），用 gpui-component TabBar 渲染。
fn database_backup_tabs(
    active: BackupTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let _ = colors;
    let selected_index = BackupTab::ALL
        .iter()
        .position(|tab| *tab == active)
        .unwrap_or(0);
    let view = cx.entity();
    BackupTab::ALL
        .iter()
        .map(|tab| Tab::from(tab.label()))
        .fold(
            TabBar::new("backup-tabs")
                .segmented()
                .small()
                .selected_index(selected_index)
                .on_click(move |index, _, cx| {
                    if let Some(tab) = BackupTab::ALL.get(*index) {
                        let _ = view.update(cx, |this, cx| {
                            this.set_backup_tab(*tab, cx);
                        });
                    }
                }),
            |bar, tab| bar.child(tab),
        )
        .into_element()
}

fn database_backup_modal_body(
    form: &BackupForm,
    file_name_input: Entity<InputState>,
    note_input: Entity<InputState>,
    object_search_input: Entity<InputState>,
    objects_scroll: &VirtualListScrollHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .p_5()
        .flex()
        .flex_col()
        .gap_3()
        .child(match form.tab {
            BackupTab::General => {
                database_backup_general_body(form, file_name_input, note_input, colors, cx)
            }
            BackupTab::Objects => {
                database_backup_objects_body(form, object_search_input, objects_scroll, colors, cx)
            }
            BackupTab::Advanced => database_backup_advanced_body(form, colors, cx),
            BackupTab::Log => database_backup_log_placeholder(colors),
        })
}

fn database_backup_general_body(
    form: &BackupForm,
    file_name_input: Entity<InputState>,
    note_input: Entity<InputState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let _ = cx; // 本页无交互监听，保留签名用于空占位。
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .gap_3()
        .child(database_backup_target_input(form.database.clone().unwrap_or_default(), colors))
        .child(database_backup_mode_info(colors))
        .child(database_backup_file_name_row(file_name_input, colors))
        .child(database_backup_note_row(note_input, colors))
        .child(div().flex_1())
}

/// 常规页「备注」输入行：随备份成功写入 {文件}.sql.meta.json，可在备份 tab 内继续编辑。
fn database_backup_note_row(input: Entity<InputState>, colors: UiColors) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(sql_file_section_label("备注（可空）", colors))
        .child(
            div()
                .h(px(34.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .overflow_hidden()
                .child(
                    Input::new(&input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .text_size(px(13.)),
                ),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("备注将保存在备份文件的元数据中，可在「备份」列表 tab 的备注列查看与编辑。"),
        )
}

fn database_backup_target_input(database: String, colors: UiColors) -> Div {
    div()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(format!("备份数据库：{database}"))
}

/// 备份方式只读说明：统一自动选择（按库类型与工具自动判断原生或逻辑），不提供手动三选一。
fn database_backup_mode_info(colors: UiColors) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(sql_file_section_label("备份方式", colors))
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors.text)
                .child("自动选择（按数据库类型与已装工具自动判断：优先原生，否则逻辑备份）"),
        )
}

fn database_backup_file_name_row(
    input: Entity<InputState>,
    colors: UiColors,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(sql_file_section_label("备份文件名（模板）", colors))
        .child(
            div()
                .h(px(34.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .overflow_hidden()
                .child(
                    Input::new(&input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .text_size(px(13.)),
                ),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("留空使用默认文件名（库名_时间戳.sql）；支持 {timestamp} / {database} 占位。"),
        )
}

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
    let selected_objects = selected_tables
        + if form.include_views { view_tables } else { 0 };
    let total_objects = form.all_table_names.len() + view_tables;

    let has_tables = !form.all_table_names.is_empty();
    // 表列表虚拟化：只渲染视口附近的行，表多时避免每帧重建整棵行 DOM（fps 掉到 20 的根因）。
    // 行高固定 40px，item_sizes 每行给 size(0, 40)；渲染期从实体内读取 selected_tables 判勾选态。
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
                    .child(if has_tables { "未找到匹配的表" } else { "暂无可备份对象" }),
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
            Rc::new(vec![size(px(0.), px(40.)); filtered_tables.len()]);
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

    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(sql_file_section_label("对象选择", colors))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(format!("已选择 {selected_objects} / {total_objects} 个对象")),
                ),
        )
        // 搜索框：左侧搜索图标、右侧输入非空时显示清除按钮。
        .child(
            div()
                .h(px(34.))
                .px_2()
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .flex()
                .items_center()
                .gap_2()
                .child(app_icon(AppIcon::Search, 15., colors.muted))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .child(Input::new(&object_search_input).small()),
                )
                .when(!form.object_search.is_empty(), |this| {
                    this.child(
                        div()
                            .id("backup-search-clear")
                            .size(px(26.))
                            .rounded(colors.radius)
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(move |style| style.bg(colors.hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    if let Some(form) = &mut this.pending_backup_modal {
                                        form.object_search = String::new();
                                    }
                                    this.backup_object_search_input.update(cx, |input, cx| {
                                        input.set_value(String::new(), window, cx);
                                    });
                                    cx.notify();
                                    cx.stop_propagation();
                                }),
                            )
                            .child(app_icon(AppIcon::Close, 14., colors.muted)),
                    )
                }),
        )
        // 批量操作工具条：全选复选框 + 结果数量 + 全选/清空按钮。
        .child(
            div()
                .h(px(32.))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Checkbox::new("backup-select-all")
                        .checked(
                            !filtered_tables.is_empty()
                                && filtered_tables.iter().all(|n| form.selected_tables.contains(n)),
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
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child("全选"),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(format!("当前 {} 项", filtered_tables.len())),
                )
                .child(div().flex_1())
                .child(
                    Button::new("backup-select-all-btn")
                        .label("全选")
                        .small()
                        .ghost()
                        .rounded(colors.radius)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_backup_filtered_tables(true, cx);
                            cx.stop_propagation();
                        })),
                )
                .child(
                    Button::new("backup-clear-btn")
                        .label("清空")
                        .small()
                        .ghost()
                        .rounded(colors.radius)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_backup_filtered_tables(false, cx);
                            cx.stop_propagation();
                        })),
                ),
        )
        // 表分组标题（固定，数量为当前筛选结果）。
        .child(database_backup_object_group_header(
            "表",
            filtered_tables.len(),
            colors,
        ))
        // 表列表：带边框、圆角、背景色的滚动容器。虚拟化列表的滚动条挂在此容器上。
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .overflow_hidden()
                .vertical_scrollbar(&objects_scroll.clone())
                .child(list_scroll),
        )
        // 视图分组：数据模型为单一开关（include_views），非逐视图勾选，故只渲染一行。
        .when(view_tables > 0, |this| {
            this.child(database_backup_object_group_header(
                "视图",
                view_tables,
                colors,
            ))
            .child(database_backup_checkbox_row(
                "backup-include-views",
                "包含视图",
                form.include_views,
                BackupCheckboxField::IncludeViews,
                colors,
                cx,
            ))
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

/// 对象分组的紧凑标题：较小字号 + 次级颜色，右侧显示数量。
fn database_backup_object_group_header(
    name: &str,
    count: usize,
    colors: UiColors,
) -> Div {
    div()
        .px_1()
        .h(px(22.))
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(name.to_string()),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(format!("({count})")),
        )
}

/// 对象选择页签：单张表的整行行。整行可点击切换选择；Checkbox 为受控展示（选中态），
/// 其 on_click 停止冒泡，避免与整行点击双重触发。
fn database_backup_table_row(
    name: &str,
    checked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let view = cx.entity();
    let name_string = name.to_string();
    div()
        .id(format!("backup-obj-row-{name_string}"))
        .h(px(40.))
        .w_full()
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .rounded(colors.radius)
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .when(checked, |d| d.bg(colors.input_bg))
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
            Checkbox::new(format!("backup-table-{name}"))
                .checked(checked)
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
            div()
                .flex_1()
                .min_w(px(0.))
                .child(
                    div()
                        .truncate()
                        .text_size(px(13.))
                        .text_color(colors.text)
                        .child(name_string),
                ),
        )
}

fn database_backup_advanced_body(
    form: &BackupForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .gap_2()
        .child(sql_file_section_label("高级选项", colors))
        .child(database_backup_checkbox_row(
            "backup-lock-tables",
            "锁定所有表",
            form.lock_tables,
            BackupCheckboxField::LockTables,
            colors,
            cx,
        ))
        .child(database_backup_checkbox_row(
            "backup-single-transaction",
            "使用单一事务（仅 InnoDB）",
            form.single_transaction,
            BackupCheckboxField::SingleTransaction,
            colors,
            cx,
        ))
        .child(database_backup_checkbox_row(
            "backup-include-routines",
            "包含存储过程/函数（--routines）",
            form.include_routines,
            BackupCheckboxField::IncludeRoutines,
            colors,
            cx,
        ))
        .child(database_backup_checkbox_row(
            "backup-include-schema",
            "包含表结构",
            form.include_schema,
            BackupCheckboxField::IncludeSchema,
            colors,
            cx,
        ))
        .child(database_backup_checkbox_row(
            "backup-include-data",
            "包含表数据",
            form.include_data,
            BackupCheckboxField::IncludeData,
            colors,
            cx,
        ))
        .child(div().flex_1())
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(
                    "「锁定所有表」「单一事务」「包含存储过程/函数」仅对 MySQL/TiDB 原生 mysqldump 生效；\
                     「包含表结构/数据」仅对逻辑备份生效。\n\
                     若报 Couldn't execute 'SELECT LIBRARY_NAME …'（mysqldump 9.x 客户端 + 老版本服务端），\
                     取消「包含存储过程/函数」或在设置中换用与服务端匹配的 mysqldump。",
                ),
        )
}

#[derive(Clone, Copy)]
enum BackupCheckboxField {
    IncludeViews,
    LockTables,
    SingleTransaction,
    IncludeRoutines,
    IncludeSchema,
    IncludeData,
}

fn database_backup_checkbox_row(
    id: &'static str,
    label: &'static str,
    checked: bool,
    field: BackupCheckboxField,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let view = cx.entity();
    div()
        .h(px(30.))
        .rounded(colors.radius)
        .flex()
        .items_center()
        .gap_2()
        .child(
            Checkbox::new(id)
                .checked(checked)
                .on_click(move |new_checked, _, cx| {
                    let value = *new_checked;
                    let field = field;
                    let _ = view.update(cx, |this, cx| {
                        match field {
                            BackupCheckboxField::IncludeViews => {
                                this.set_backup_include_views(value, cx)
                            }
                            BackupCheckboxField::LockTables => this.set_backup_lock_tables(value, cx),
                            BackupCheckboxField::SingleTransaction => {
                                this.set_backup_single_transaction(value, cx)
                            }
                            BackupCheckboxField::IncludeRoutines => {
                                this.set_backup_include_routines(value, cx)
                            }
                            BackupCheckboxField::IncludeSchema => {
                                this.set_backup_include_schema(value, cx)
                            }
                            BackupCheckboxField::IncludeData => this.set_backup_include_data(value, cx),
                        }
                    });
                }),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors.text)
                .child(label),
        )
}

fn database_backup_log_placeholder(colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors.muted)
                .child("开始备份后，将在此查看执行日志。"),
        )
}

fn elapsed_seconds(started_at: Instant, finished_at: Option<Instant>) -> u64 {
    finished_at
        .unwrap_or_else(Instant::now)
        .saturating_duration_since(started_at)
        .as_secs()
}

fn database_backup_task_card(
    task: &BackupTaskState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let filename = task
        .output_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let status = if task.canceled {
        "已取消".to_string()
    } else if let Some(error) = &task.error {
        format!("失败：{error}")
    } else if task.running() {
        format!("执行中：{}", task.stage)
    } else if task.skipped.is_empty() {
        "完成".to_string()
    } else {
        format!("完成（跳过 {} 项）", task.skipped.len())
    };
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(filename),
                        )
                        // 状态文本（含失败原因）可能很长：自动换行完整展示，避免单行截断。
                        .child(
                            div()
                                .flex_none()
                                .ml_2()
                                .text_size(px(12.))
                                .text_right()
                                .whitespace_normal()
                                .text_color(if task.error.is_some() {
                                    rgb(0xd64545)
                                } else {
                                    colors.muted
                                })
                                .child(status),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(format!("{} · {}", task.database.clone().unwrap_or_default(), task.mode.label()))
                        .child("·")
                        .child(format!("用时 {}s", elapsed_seconds(task.started_at, task.finished_at))),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .overflow_y_scrollbar()
                .child(database_backup_log_list(task, colors, cx)),
        )
}

/// 备份日志列表：长文本（如 mysqldump 报错）自动换行；每行可复制，顶部可复制全部。
/// gpui-component 无 selectable 文本控件，复制走剪贴板按钮（与仓库其他模块一致）。
fn database_backup_log_list(
    task: &BackupTaskState,
    colors: UiColors,
    _cx: &mut Context<NavicatMain>,
) -> Div {
    // 单行复制格式与展示一致：`[阶段] 消息`；全部复制按行拼接。
    let format_entry = |stage: &str, message: &str| format!("[{stage}] {message}");
    let all_text = task
        .logs
        .iter()
        .map(|entry| format_entry(&entry.stage, &entry.message))
        .chain(
            task.skipped
                .iter()
                .map(|skipped| format_entry("跳过", skipped)),
        )
        .collect::<Vec<_>>()
        .join("\n");
    let mut list = div().flex().flex_col().gap_1().p_3();
    if task.logs.is_empty() && task.skipped.is_empty() {
        list = list.child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("正在执行，消息日志会实时追加…"),
        );
    } else {
        // 顶部工具行：复制全部日志（含失败原因与跳过项）。
        list = list.child(
            div()
                .flex()
                .justify_end()
                .pb_1()
                .child(backup_text_button("复制全部", colors, {
                    let all_text = all_text.clone();
                    move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(all_text.clone()));
                    }
                })),
        );
        let hidden_count = task.logs.len().saturating_sub(BACKUP_VISIBLE_LOG_LIMIT);
        if hidden_count > 0 {
            list = list.child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child(format!("以上 {hidden_count} 条日志已隐藏")),
            );
        }
        // 日志行：阶段列固定宽度、消息列弹性并允许换行、行尾复制按钮。
        for entry in task.logs.iter().skip(hidden_count) {
            let copy_text = format_entry(&entry.stage, &entry.message);
            list = list.child(backup_log_row(
                entry.stage.clone(),
                entry.message.clone(),
                entry.success,
                copy_text,
                colors,
            ));
        }
        for skipped in &task.skipped {
            let copy_text = format_entry("跳过", skipped);
            list = list.child(backup_log_row(
                "跳过".to_string(),
                skipped.clone(),
                false,
                copy_text,
                colors,
            ));
        }
    }
    list
}

/// 单条日志行：`[阶段] 消息`，消息超长自动换行，行尾提供复制到剪贴板。
fn backup_log_row(
    stage: String,
    message: String,
    success: bool,
    copy_text: String,
    colors: UiColors,
) -> Div {
    div()
        .flex()
        .items_start()
        .gap_2()
        .text_size(px(12.))
        .text_color(if success { colors.text } else { rgb(0xd64545) })
        .child(div().flex_none().child(format!("[{stage}]")))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .whitespace_normal()
                .child(message),
        )
        .child(div().flex_none().child(backup_text_button(
            "复制",
            colors,
            move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
            },
        )))
}

fn database_backup_modal_footer(
    can_start: bool,
    log_task: Option<u64>,
    tasks: &[BackupTaskState],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut footer = div()
        .h(px(54.))
        .flex_none()
        .px_5()
        .border_t_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .child(
            Button::new("backup-close")
                .label("关闭")
                .small()
                .w(px(78.))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.cancel_backup_modal(cx);
                    cx.stop_propagation();
                })),
        );
    if let Some(task_id) = log_task {
        let task = tasks.iter().find(|task| task.id == task_id);
        let finished = task.map(|task| !task.running()).unwrap_or(true);
        let cancel_requested = task.map(|task| task.cancel_requested).unwrap_or(false);
        if !finished {
            footer = footer.child(
                Button::new("backup-cancel-task")
                    .label(if cancel_requested { "停止中" } else { "取消任务" })
                    .small()
                    .w(px(92.))
                    .disabled(cancel_requested)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.cancel_backup_task(task_id, cx);
                        cx.stop_propagation();
                    })),
            );
        } else {
            // 任务已完成：保留在菜单里的同时，提供「再次备份」直接回到新建表单（开始备份按钮）。
            if let Some(task) = task {
                let reconn = task.connection_id;
                let redb = task.database.clone();
                footer = footer.child(
                    Button::new("backup-again")
                        .label("再次备份")
                        .small()
                        .w(px(92.))
                        .primary()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.show_backup_modal(Some(reconn), redb.clone(), window, cx);
                            cx.stop_propagation();
                        })),
                );
            }
            footer = footer.child(
                Button::new("backup-clear-task")
                    .label("清除任务")
                    .small()
                    .w(px(92.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.clear_backup_task(task_id, cx);
                        cx.stop_propagation();
                    })),
            );
        }
    } else {
        let running = tasks.iter().any(|task| task.running());
        footer = footer.child(
            Button::new("backup-start")
                .label("开始备份")
                .primary()
                .small()
                .w(px(96.))
                .disabled(!can_start || running)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_backup(cx);
                    cx.stop_propagation();
                })),
        );
    }
    footer
}
