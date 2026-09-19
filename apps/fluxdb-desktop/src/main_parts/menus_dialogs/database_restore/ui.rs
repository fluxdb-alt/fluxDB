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
        "目标新文件"
    } else {
        "目标数据库名称"
    };
    // 「现有库」模式：目标库走下拉（目标连接下的已存在库），不新建。
    let existing_mode = modal.kind != DatabaseKind::Sqlite
        && modal.mode.read(cx).selected_value().is_some_and(|v| v == "现有库");
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
                    "目标连接",
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
                    "备份文件",
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
                                .label("选择")
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
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("按文件内容识别格式；SQLite 备份恢复为 .db 新文件。"),
        )
        // 目标库名称/文件：「现有库」-> 下拉；「新数据库」/Sqlite -> 文本框。
        .child({
            let label = if existing_mode {
                "目标数据库"
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
                    "恢复方式",
                    colors,
                )))
                .child(Select::new(&modal.mode).disabled(locked).w_full()),
        )
        .child(
            div()
                .pl(px(132.))
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("先「预检查」生成恢复计划并确认，才能「确认恢复」；非空目标禁止覆盖。"),
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
    let existing_mode =
        !is_sqlite && modal.mode.read(cx).selected_value().is_some_and(|v| v == "现有库");
    let count = modal.table_grid.len();

    // 单元格文案：备份内容与目标状态直接取探测事实。
    let body = if modal.table_grid.is_empty() {
        let note: Option<String> = if is_sqlite {
            Some("SQLite 恢复为完整数据库快照（新文件），无需逐对象设置。".to_string())
        } else if !existing_mode {
            Some(
                "整库恢复到新数据库：将还原备份内全部对象。切换到「现有库」可逐对象设置还原动作。"
                    .to_string(),
            )
        } else if modal.probe_running {
            None // 探测中：单独渲染 loading。
        } else if let Some(err) = &modal.probe_error {
            Some(format!("目标检查失败：{err}。请调整备份文件或目标库后自动重试。"))
        } else if modal
            .source_input
            .read(cx)
            .value()
            .trim()
            .is_empty()
            || modal.target_db.read(cx).selected_value().is_none_or(|s| s.is_empty())
        {
            Some("先选择备份文件与目标数据库，将自动检查对象存在性与备份内容。".to_string())
        } else {
            Some("备份文件中未解析到可逐对象处理的表。".to_string())
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
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child("正在检查目标对象与备份内容…"),
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
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(note.unwrap_or_default()),
                )
                .into_any_element()
        }
    } else {
        let rows = modal.table_grid.iter().map(|row| {
            let status = if row.exists { "已存在" } else { "不存在" };
            let content = match (row.has_ddl, row.has_data) {
                (true, true) => "结构＋数据",
                (true, false) => "仅结构",
                (false, true) => "仅数据",
                (false, false) => "空对象",
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
                        .text_size(px(13.))
                        .text_color(colors.text)
                        .child(row.name.clone()),
                )
                .child(
                    div()
                        .w(px(84.))
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(content),
                )
                .child(
                    div()
                        .w(px(60.))
                        .flex_none()
                        .text_size(px(11.))
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
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child("对象"),
                    )
                    .child(
                        div()
                            .w(px(84.))
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child("备份内容"),
                    )
                    .child(
                        div()
                            .w(px(60.))
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child("目标状态"),
                    )
                    .child(
                        div()
                            .w(px(140.))
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child("还原动作"),
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
                .child(sql_file_section_label("对象与还原动作", colors))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(if count == 0 && existing_mode && modal.probe_running {
                            "检查中…".to_string()
                        } else {
                            format!("共 {count} 个对象")
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
                .child(sql_file_section_label("执行选项", colors))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child("SQLite 为整库文件快照恢复，无事务范围与逐表行数选项。"),
                )
        } else {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(sql_file_section_label("执行选项", colors))
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
                                .child(sql_file_section_label("事务范围", colors)),
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
                                .child(sql_file_section_label("完成验证", colors)),
                        )
                        .child(Select::new(&modal.validation).disabled(locked).w_full()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child("选项在「预检查」时生效；实际兑现范围由引擎能力决定，无法原子回滚时会在计划中说明。修改后需重新预检查。"),
                )
        })
        .child(match &runtime.prepared {
            None => div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("先在底栏「预检查」生成计划；此处展示实际执行策略、影响范围与风险。"),
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
        .map(|t| format!("{}（{}）", t.name, t.default_action.label()))
        .collect();
    let modified = plan
        .source_modified
        .map(|t| {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            dt.format("%Y-%m-%d %H:%M:%S").to_string()
        })
        .unwrap_or_else(|| "未知".to_string());

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
                .child(sql_file_section_label("来源", colors))
                .child(restore_kv(
                    "格式",
                    plan.format.label(),
                    colors,
                ))
                .child(restore_kv(
                    "大小",
                    &format_backup_size(plan.source_size),
                    colors,
                ))
                .child(restore_kv("修改时间", &modified, colors))
                .child(restore_kv("范围", &plan.summary, colors)),
        )
        // 动作统计
        .when(!plan.tables.is_empty(), |this| {
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(sql_file_section_label("按动作统计", colors))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.text)
                            .child(format!(
                                "新建 {create} · 重建 {recreate} · 清空后导入 {truncate} · 追加 {append} · 不处理 {skip}"
                            )),
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
                    .child(sql_file_section_label("将删除原数据", colors))
                    .children(destructive.iter().map(|item| {
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(app_icon(AppIcon::Trash, 13., rgb(0xe5484d)))
                            .child(
                                div()
                                    .text_size(px(12.))
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
                    .child(sql_file_section_label("风险", colors))
                    .children(plan.tables.iter().flat_map(|t| {
                        t.risks.iter().map(move |r| {
                            div()
                                .text_size(px(12.))
                                .text_color(colors.muted)
                                .child(format!("· {}：{}", t.name, r))
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
                    .child(sql_file_section_label("警告", colors))
                    .children(plan.warnings.iter().map(|w| {
                        div()
                            .text_size(px(12.))
                            .text_color(colors.muted)
                            .child(format!("· {w}"))
                    })),
            )
        })
        // 执行策略说明（引擎能力决定，只读）
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(sql_file_section_label("执行策略", colors))
                .child(restore_kv("错误处理", "遇错停止，保留已完成结果清单", colors))
                .child(restore_kv(
                    "事务",
                    txn.label(),
                    colors,
                ))
                .child(restore_kv(
                    "清空实现",
                    "由执行器决定：MySQL DELETE（临时关闭外键检查）/ PostgreSQL TRUNCATE RESTART IDENTITY",
                    colors,
                ))
                .child(restore_kv("完成验证", val.label(), colors)),
        )
}

/// 只读键值行：120px 标签列 + 弹性值列，与常规页对齐。
fn restore_kv(key: &'static str, value: &str, colors: UiColors) -> Div {
    div()
        .w_full()
        .flex()
        .items_start()
        .gap_3()
        .child(
            div()
                .w(px(72.))
                .flex_none()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(key),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(12.))
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
        .child(sql_file_section_label("消息日志", colors))
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
