// 恢复弹框分页渲染（常规 / 对象选择 / 高级 / 消息日志），与备份弹框同款四页结构（设计文档 §14.1）。
// 纯 UI + 页签 setter；预检查/执行逻辑与弹框骨架在 database_restore.rs。

impl NavicatMain {
    fn set_restore_tab(&mut self, tab: RestoreTab, cx: &mut Context<Self>) {
        if let Some(modal) = &mut self.restore_modal {
            modal.tab = tab;
            cx.notify();
        }
    }
}

/// 四页签（Navicat 风格），复用备份弹框的 TabBar 渲染。
fn restore_modal_tabs(
    active: RestoreTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let _ = colors;
    let selected_index = RestoreTab::ALL
        .iter()
        .position(|tab| *tab == active)
        .unwrap_or(0);
    let view = cx.entity();
    RestoreTab::ALL
        .iter()
        .map(|tab| Tab::from(tab.label()))
        .fold(
            TabBar::new("restore-tabs")
                .segmented()
                .small()
                .selected_index(selected_index)
                .on_click(move |index, _, cx| {
                    if let Some(tab) = RestoreTab::ALL.get(*index) {
                        let _ = view.update(cx, |this, cx| {
                            this.set_restore_tab(*tab, cx);
                        });
                    }
                }),
            |bar, tab| bar.child(tab),
        )
        .into_element()
}

/// 常规页：目标连接、备份文件、目标库/文件、恢复方式（设计文档 §6.2）。
fn restore_general_page(
    modal: &RestoreModal,
    locked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let target_label = if modal.kind == DatabaseKind::Sqlite {
        tr("app.database_restore.target.new_file")
    } else {
        tr("app.database_restore.target.database_name")
    };
    // 「现有库」模式：目标库走下拉（目标连接下的已存在库），不新建。
    let existing_mode = modal.kind != DatabaseKind::Sqlite
        && modal
            .mode
            .read(cx)
            .selected_value()
            .is_some_and(|v| *v == RestoreTargetMode::ExistingDatabase);
    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .px_5()
        .py_4()
        .flex()
        .flex_col()
        .gap_3()
        // 目标连接
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .child(div().w(px(120.)).flex_none().child(sql_file_section_label(
                    tr("app.database_restore.field.target_connection"),
                    colors,
                )))
                .child(Select::new(&modal.connection).disabled(locked).w_full()),
        )
        // 备份文件完整路径：输入框 + 「选择」按钮（原生打开设置备份目录）
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .child(div().w(px(120.)).flex_none().child(sql_file_section_label(
                    tr("app.database_restore.field.backup_file"),
                    colors,
                )))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(restore_input_frame(modal.source_input.clone(), colors))
                        .child(
                            Button::new("restore-pick-backup-file")
                                .label(tr("app.common.choose"))
                                .small()
                                .rounded(colors.radius)
                                .disabled(locked)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.choose_restore_backup_file(window, cx);
                                })),
                        ),
                ),
        )
        .child(
            div()
                .pl(px(132.))
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.database_restore.hint.format_detect")),
        )
        // 目标库名称/文件：「现有库」-> 下拉；「新数据库」/Sqlite -> 文本框。
        .child({
            let label = if existing_mode {
                tr("app.database_restore.target.existing")
            } else {
                target_label
            };
            let content = if existing_mode {
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(Select::new(&modal.target_db).disabled(locked).w_full())
            } else {
                restore_input_frame(modal.target_input.clone(), colors)
            };
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .child(div().w(px(120.)).flex_none().child(sql_file_section_label(
                    label,
                    colors,
                )))
                .child(content)
        })
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .child(div().w(px(120.)).flex_none().child(sql_file_section_label(
                    tr("app.database_restore.field.mode"),
                    colors,
                )))
                .child(Select::new(&modal.mode).disabled(locked).w_full()),
        )
        .child(
            div()
                .pl(px(132.))
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.database_restore.hint.preflight")),
        )
        .child(div().flex_1())
}

/// 对象选择页：源对象清单 + 每对象还原动作（设计文档 §6.3）。
/// 进入现有库模式即后台探测目标：备份内容与目标状态来自探测事实，先于「预检查」呈现，
/// 动作候选按存在性/内容过滤；已存在对象默认「待选择策略」，未选定前不能预检查。
fn restore_objects_page(
    modal: &RestoreModal,
    _runtime: &RestoreRuntime,
    locked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let is_sqlite = modal.kind == DatabaseKind::Sqlite;
    let existing_mode = !is_sqlite
        && modal
            .mode
            .read(cx)
            .selected_value()
            .is_some_and(|v| *v == RestoreTargetMode::ExistingDatabase);
    let count = modal.table_grid.len();

    // 单元格文案：备份内容与目标状态直接取探测事实。
    let body = if modal.table_grid.is_empty() {
        let note: Option<String> = if is_sqlite {
            Some(tr("app.database_restore.objects.sqlite_note").to_string())
        } else if !existing_mode {
            Some(tr("app.database_restore.objects.whole_db_note").to_string())
        } else if modal.probe_running {
            None // 探测中：单独渲染 loading。
        } else if let Some(err) = &modal.probe_error {
            Some(
                tr("app.database_restore.objects.probe_failed")
                    .replace("%{error}", err)
                    .to_string(),
            )
        } else if modal
            .source_input
            .read(cx)
            .value()
            .trim()
            .is_empty()
            || modal.target_db.read(cx).selected_value().is_none_or(|s| s.is_empty())
        {
            Some(tr("app.database_restore.objects.need_source_target").to_string())
        } else {
            Some(tr("app.database_restore.objects.no_tables").to_string())
        };
        if note.is_none() {
            // 探测中状态。
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(app_icon(AppIcon::Refresh, 22., colors.muted))
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(colors.muted)
                        .child(tr("app.database_restore.objects.checking")),
                )
                .into_any_element()
        } else {
            let icon = if modal.probe_error.is_some() && !is_sqlite && existing_mode {
                AppIcon::CircleSlash
            } else {
                AppIcon::List
            };
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(app_icon(icon, 22., colors.muted))
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(colors.muted)
                        .child(note.unwrap_or_default()),
                )
                .into_any_element()
        }
    } else {
        let rows = modal.table_grid.iter().map(|row| {
            let status = if row.exists {
                tr("app.database_restore.objects.status.exists")
            } else {
                tr("app.database_restore.objects.status.missing")
            };
            let content = match (row.has_ddl, row.has_data) {
                (true, true) => tr("app.database_restore.objects.content.structure_data"),
                (true, false) => tr("app.database_restore.objects.content.structure_only"),
                (false, true) => tr("app.database_restore.objects.content.data_only"),
                (false, false) => tr("app.database_restore.objects.content.empty"),
            };
            div()
                .w_full()
                .h(px(34.))
                .flex_none()
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.text)
                        .child(row.name.clone()),
                )
                .child(
                    div()
                        .w(px(84.))
                        .flex_none()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(content),
                )
                .child(
                    div()
                        .w(px(60.))
                        .flex_none()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(status),
                )
                .child(
                    div()
                        .w(px(140.))
                        .flex_none()
                        .child(Select::new(&row.select).disabled(locked).w_full()),
                )
        });
        div()
            .flex_1()
            .min_h(px(0.))
            .rounded(colors.radius)
            .border_1()
            .border_color(colors.border)
            .bg(colors.input_bg)
            // 单一滚动容器（与备份对象列表同款）：表头 + 数据行同处一个滚动列，
            // 不再嵌套 scroller，避免内层滚动区在 overflow_hidden 下高度塌成 0 而裁掉行。
            .overflow_y_scrollbar()
            .flex()
            .flex_col()
            // 表头：与行同列宽对齐。
            .child(
                div()
                    .w_full()
                    .h(px(28.))
                    .flex_none()
                    .px_2()
                    .border_b_1()
                    .border_color(colors.border)
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_size(ui_text_size(11.))
                            .text_color(colors.muted)
                            .child(tr("app.database_restore.objects.column.object")),
                    )
                    .child(
                        div()
                            .w(px(84.))
                            .flex_none()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.muted)
                            .child(tr("app.database_restore.objects.column.content")),
                    )
                    .child(
                        div()
                            .w(px(60.))
                            .flex_none()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.muted)
                            .child(tr("app.database_restore.objects.column.target_state")),
                    )
                    .child(
                        div()
                            .w(px(140.))
                            .flex_none()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.muted)
                            .child(tr("app.database_restore.objects.column.action")),
                    ),
            )
            .children(rows)
            .into_any_element()
    };

    div()
        .flex_1()
        .min_h(px(0.))
        .px_5()
        .py_4()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .child(sql_file_section_label(
                    tr("app.database_restore.objects.title"),
                    colors,
                ))
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(if count == 0 && existing_mode && modal.probe_running {
                            tr("app.database_restore.objects.checking_short").to_string()
                        } else {
                            tr("app.database_restore.objects.count")
                                .replace("%{count}", &count.to_string())
                        }),
                ),
        )
        .child(body)
}

/// 高级页：上半为真实执行选项（事务范围/完成验证，兑现到执行链），下半为预检查计划、
/// 影响范围与风险（设计文档 §6.7/§6.8）。SQLite 快照无这两项语义，故隐藏控件。
fn restore_advanced_page(
    modal: &RestoreModal,
    runtime: &RestoreRuntime,
    locked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let is_sqlite = modal.kind == DatabaseKind::Sqlite;
    // 当前选择（按选中行索引回枚举），供计划预览的「执行策略」如实回显。
    let txn = match modal.transaction.read(cx).selected_index(cx).map(|p| p.row) {
        Some(1) => fluxdb_app::RestoreTransactionMode::SingleTransaction,
        _ => fluxdb_app::RestoreTransactionMode::EngineDefault,
    };
    let val = match modal.validation.read(cx).selected_index(cx).map(|p| p.row) {
        Some(1) => fluxdb_app::RestoreValidation::RowCount,
        _ => fluxdb_app::RestoreValidation::Basic,
    };
    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .px_5()
        .py_4()
        .flex()
        .flex_col()
        .gap_3()
        .child(if is_sqlite {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(sql_file_section_label(
                    tr("app.database_restore.advanced.options_title"),
                    colors,
                ))
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(colors.muted)
                        .child(tr("app.database_restore.advanced.sqlite_note")),
                )
        } else {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(sql_file_section_label(
                    tr("app.database_restore.advanced.options_title"),
                    colors,
                ))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .w(px(120.))
                                .flex_none()
                                .child(sql_file_section_label(
                                    tr("app.database_restore.advanced.transaction"),
                                    colors,
                                )),
                        )
                        .child(Select::new(&modal.transaction).disabled(locked).w_full()),
                )
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .w(px(120.))
                                .flex_none()
                                .child(sql_file_section_label(
                                    tr("app.database_restore.advanced.validation"),
                                    colors,
                                )),
                        )
                        .child(Select::new(&modal.validation).disabled(locked).w_full()),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(tr("app.database_restore.advanced.options_hint")),
                )
        })
        .child(match &runtime.prepared {
            None => div()
                .text_size(ui_text_size(12.))
                .text_color(colors.muted)
                .child(tr("app.database_restore.advanced.need_plan")),
            Some((_, plan)) => restore_plan_preview(plan, txn, val, colors),
        })
}

/// 计划预览（设计文档 §6.8）：来源、按动作统计、将删除原数据的对象、警告与逐表风险。
fn restore_plan_preview(
    plan: &fluxdb_app::RestorePlan,
    txn: fluxdb_app::RestoreTransactionMode,
    val: fluxdb_app::RestoreValidation,
    colors: UiColors,
) -> Div {
    use fluxdb_core::RestoreTableAction as Act;
    // 按动作统计（仅逐表模式有 tables；整库模式为 0，改由 summary 说明）。
    let (mut create, mut recreate, mut truncate, mut append, mut skip) = (0, 0, 0, 0, 0);
    for t in &plan.tables {
        match t.default_action {
            Act::Create => create += 1,
            Act::Recreate => recreate += 1,
            Act::TruncateAndLoad => truncate += 1,
            Act::Append => append += 1,
            Act::Skip => skip += 1,
        }
    }
    let destructive: Vec<String> = plan
        .tables
        .iter()
        .filter(|t| t.default_action.destroys_target_data())
        .map(|t| {
            tr("app.database_restore.plan.object_with_action")
                .replace("%{name}", &t.name)
                .replace("%{action}", restore_action_label(t.default_action).as_str())
        })
        .collect();
    let modified = plan
        .source_modified
        .map(|t| {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            dt.format("%Y-%m-%d %H:%M:%S").to_string()
        })
        .unwrap_or_else(|| tr("app.database_restore.plan.unknown").to_string());

    div()
        .flex()
        .flex_col()
        .gap_3()
        // 来源摘要
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(sql_file_section_label(
                    tr("app.database_restore.plan.source"),
                    colors,
                ))
                .child(restore_kv(
                    tr("app.database_restore.plan.format"),
                    restore_format_label(plan.format).as_str(),
                    colors,
                ))
                .child(restore_kv(
                    tr("app.database_restore.plan.size"),
                    &format_backup_size(plan.source_size),
                    colors,
                ))
                .child(restore_kv(
                    tr("app.database_restore.plan.modified"),
                    &modified,
                    colors,
                ))
                .child(restore_kv(
                    tr("app.database_restore.plan.scope"),
                    &plan.summary,
                    colors,
                )),
        )
        // 动作统计
        .when(!plan.tables.is_empty(), |this| {
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(sql_file_section_label(
                        tr("app.database_restore.plan.by_action"),
                        colors,
                    ))
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(colors.text)
                            .child(
                                tr("app.database_restore.plan.action_counts")
                                    .replace("%{create}", &create.to_string())
                                    .replace("%{recreate}", &recreate.to_string())
                                    .replace("%{truncate}", &truncate.to_string())
                                    .replace("%{append}", &append.to_string())
                                    .replace("%{skip}", &skip.to_string()),
                            ),
                    ),
            )
        })
        // 将删除原数据的对象（破坏性）
        .when(!destructive.is_empty(), |this| {
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(sql_file_section_label(
                        tr("app.database_restore.plan.destructive_title"),
                        colors,
                    ))
                    .children(destructive.iter().map(|item| {
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(app_icon(AppIcon::Trash, 13., rgb(0xe5484d)))
                            .child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .text_color(colors.text)
                                    .child(item.clone()),
                            )
                    })),
            )
        })
        // 逐表风险
        .when(plan.tables.iter().any(|t| !t.risks.is_empty()), |this| {
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(sql_file_section_label(
                        tr("app.database_restore.plan.risks"),
                        colors,
                    ))
                    .children(plan.tables.iter().flat_map(|t| {
                        t.risks.iter().map(move |r| {
                            div()
                                .text_size(ui_text_size(12.))
                                .text_color(colors.muted)
                                .child(
                                    tr("app.database_restore.plan.risk_line")
                                        .replace("%{name}", &t.name)
                                        .replace("%{risk}", r),
                                )
                        })
                    })),
            )
        })
        // 计划警告
        .when(!plan.warnings.is_empty(), |this| {
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(sql_file_section_label(
                        tr("app.database_restore.plan.warnings"),
                        colors,
                    ))
                    .children(plan.warnings.iter().map(|w| {
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(colors.muted)
                                .child(
                                    tr("app.database_restore.plan.warning_line")
                                        .replace("%{warning}", w),
                                )
                    })),
            )
        })
        // 执行策略说明（引擎能力决定，只读）
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(sql_file_section_label(
                    tr("app.database_restore.plan.strategy"),
                    colors,
                ))
                .child(restore_kv(
                    tr("app.database_restore.plan.error_handling"),
                    tr("app.database_restore.plan.error_handling_value").as_str(),
                    colors,
                ))
                .child(restore_kv(
                    tr("app.database_restore.plan.transaction"),
                    restore_transaction_label(txn).as_str(),
                    colors,
                ))
                .child(restore_kv(
                    tr("app.database_restore.plan.truncate_impl"),
                    tr("app.database_restore.plan.truncate_impl_value").as_str(),
                    colors,
                ))
                .child(restore_kv(
                    tr("app.database_restore.plan.validation"),
                    restore_validation_label(val).as_str(),
                    colors,
                )),
        )
}

/// 只读键值行：120px 标签列 + 弹性值列，与常规页对齐。
fn restore_kv(key: impl Into<SharedString>, value: &str, colors: UiColors) -> Div {
    let key = key.into();
    div()
        .w_full()
        .flex()
        .items_start()
        .gap_3()
        .child(
            div()
                .w(px(72.))
                .flex_none()
                .text_size(ui_text_size(12.))
                .text_color(colors.muted)
                .child(key),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(ui_text_size(12.))
                .text_color(colors.text)
                .whitespace_normal()
                .child(value.to_string()),
        )
}

/// 消息日志页：预检查计划/警告与执行进度都在此追加（与备份任务日志同款边框区域）。
fn restore_log_page(runtime: &RestoreRuntime, colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .px_5()
        .py_4()
        .flex()
        .flex_col()
        .gap_2()
        .child(sql_file_section_label(
            tr("app.database_restore.log.title"),
            colors,
        ))
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .overflow_y_scrollbar()
                .child(restore_log_list(&runtime.logs, colors)),
        )
}
