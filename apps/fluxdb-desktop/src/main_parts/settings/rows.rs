// 这里只统一设置行的布局与背景，实际操作仍由 Button/Input/Checkbox/Switch 等组件承担。
// Form::Field 没有整行 hover 接口，ListItem 会接管选中/hover 样式；因此保留布局容器。
// 稳定 ID 让框架只在 hover 进出时通知重绘，行内移动不额外刷新，也不添加点击行为。
fn settings_row_container(id: &'static str, colors: UiColors) -> Stateful<Div> {
    settings_row_container_dynamic(format!("settings-row:{id}"), colors)
}

fn settings_row_container_dynamic(
    id: impl Into<gpui::ElementId>,
    colors: UiColors,
) -> Stateful<Div> {
    div()
        .id(id)
        .border_t_1()
        .border_color(colors.border_soft)
        .px_3()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .hover(move |style| style.bg(colors.hover))
}

fn settings_database_support_row(entry: DatabaseSupportEntry, colors: UiColors) -> Stateful<Div> {
    settings_row_container(database_kind_name(entry.kind), colors)
        .min_h(px(58.))
        .py_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w(px(0.))
                .child(
                    div()
                        .size(px(30.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border_soft)
                        .bg(colors.panel_alt)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(img(database_kind_icon_path(entry.kind)).size(px(20.))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(ui_text_size(13.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(colors.text)
                                        .child(database_kind_name(entry.kind)),
                                )
                                .child(
                                    div()
                                        .text_size(ui_text_size(11.))
                                        .text_color(colors.muted)
                                        .child(entry.source),
                                ),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(entry.detail),
                        ),
                ),
        )
        .child(settings_database_support_status(colors))
}

fn settings_database_support_status(colors: UiColors) -> impl IntoElement {
    let label = tr("app.settings.status.supported");
    settings_status_chip(label.clone(), label, colors)
}

fn settings_row_label(
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    icon: AppIcon,
    colors: UiColors,
) -> Div {
    let title = title.into();
    let detail = detail.into();
    div()
        .flex()
        .items_center()
        .gap_2()
        .min_w(px(0.))
        .child(app_icon_box(icon, 24., 13., colors.muted).flex_none())
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
                        .child(title),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(detail),
                ),
        )
}

fn settings_action_row_with_id(
    id: &'static str,
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    icon: AppIcon,
    colors: UiColors,
) -> Stateful<Div> {
    let title = title.into();
    let detail = detail.into();
    settings_row_container(id, colors)
        .h(px(52.))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w(px(0.))
                .child(app_icon_box(icon, 24., 13., colors.muted).flex_none())
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
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(detail),
                        ),
                ),
        )
}

/// 系统设置-性能诊断：右上角 FPS / 帧耗时 / CPU / GPU / 内存 悬浮 HUD 开关。
fn settings_performance_diagnostics_row(
    checked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    // Switch 的 on_click 不带 `Context<NavicatMain>`，只能拿到 `&mut App`；
    // 因此通过 self 的弱实体在 App 上下文中更新 draft，走与其它设置一致的保存流。
    let this = cx.entity().downgrade();
    settings_action_row_with_id(
        "settings-performance-diagnostics",
        tr("app.settings.system.performance"),
        tr("app.settings.system.performance_detail"),
        AppIcon::Activity,
        colors,
    )
    .child(
        Switch::new("settings-performance-diagnostics")
            .checked(checked)
            .on_click(move |checked, _, cx| {
                // render 期间 self 必然存活，失败仅可能因视图临时不可达；忽略即可。
                let _ = this.update(cx, |this, cx| {
                    this.settings_editor_draft.performance_diagnostics = *checked;
                    cx.notify();
                });
            }),
    )
}

fn settings_log_level_row(
    current: LogLevel,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    settings_action_row_with_id(
        "settings-log-level",
        tr("app.settings.system.log_level"),
        tr("app.settings.system.log_level_detail"),
        AppIcon::List,
        colors,
    )
    .child(
        div()
            .flex()
            .items_center()
            .gap_1()
            .children(
                [
                    ("Error", LogLevel::Error),
                    ("Warn", LogLevel::Warn),
                    ("Info", LogLevel::Info),
                    ("Debug", LogLevel::Debug),
                    ("Trace", LogLevel::Trace),
                ]
                .into_iter()
                .map(|(label, level)| {
                    Button::new(("settings-log-level", settings_status_id("日志级别", label)))
                        .label(label)
                        .small()
                        .rounded(colors.radius)
                        .when(current == level, |button| button.primary())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.settings_editor_draft.log_level = level;
                            cx.notify();
                        }))
                }),
            ),
    )
}

fn settings_log_path_row(
    log_path: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let display_path = configured_log_dir(log_path).display().to_string();
    settings_action_row_with_id(
        "settings-log-path",
        tr("app.settings.system.log_path"),
        tr("app.settings.system.log_path_detail"),
        AppIcon::Folder,
        colors,
    )
    .child(
        h_flex()
            .min_w(px(0.))
            .max_w(px(380.))
            .gap_2()
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .truncate()
                    .child(display_path),
            )
            .child(
                Button::new("settings-log-path")
                    .label(tr("app.common.choose"))
                    .small()
                    .rounded(colors.radius)
                    .on_click(cx.listener(|this, _, window, cx| {
                        settings_choose_log_directory(this, window, cx);
                    })),
            ),
    )
}

fn settings_choose_log_directory(
    this: &mut NavicatMain,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some(tr("app.settings.system.choose_log_dir")),
    });
    this._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
        let result = receiver.await;
        let _ = cx.update(|_window, cx| {
            let Some(view) = view.upgrade() else {
                return;
            };
            view.update(cx, |this, cx| match result {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        this.settings_editor_draft.log_path = path.display().to_string();
                        cx.notify();
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => this.show_message(
                    format!(
                        "{}: {error}",
                        tr("app.settings.system.choose_log_dir_failed")
                    ),
                    AppMessageKind::Error,
                    cx,
                ),
                Err(error) => this.show_message(
                    format!(
                        "{}: {error}",
                        tr("app.settings.system.choose_log_dir_failed")
                    ),
                    AppMessageKind::Error,
                    cx,
                ),
            });
        });
    }));
}

#[allow(
    clippy::too_many_arguments,
    reason = "设置行参数逐项对应 GPUI 控件契约，打包成结构体反而隐藏调用点差异"
)]
fn settings_choice_row_with_id(
    id: &'static str,
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    icon: AppIcon,
    current: u64,
    choices: &[(SharedString, u64)],
    apply: fn(&mut Settings, u64),
    colors: UiColors,
    preview: bool,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let title = title.into();
    let detail = detail.into();
    settings_row_container(id, colors)
        .min_h(px(52.))
        .py_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w(px(0.))
                .child(app_icon_box(icon, 24., 13., colors.muted).flex_none())
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
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(detail),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .children(choices.iter().map(move |(label, value)| {
                    let value = *value;
                    let active = current == value;
                    Button::new(SharedString::from(format!("settings-choice:{id}:{value}")))
                        .label(label.clone())
                        .small()
                        .rounded(colors.radius)
                        .when(active, |button| button.primary())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            apply(&mut this.settings_editor_draft, value);
                            if preview {
                                this.preview_settings(cx);
                            }
                            cx.notify();
                        }))
                })),
        )
}

fn settings_checkbox_row(
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    icon: AppIcon,
    id: &'static str,
    checked: bool,
    colors: UiColors,
) -> Stateful<Div> {
    let title = title.into();
    let detail = detail.into();
    settings_row_container(id, colors)
        .h(px(52.))
        .cursor_pointer()
        .child(settings_row_label(title, detail, icon, colors))
        .child(Checkbox::new(id).checked(checked))
}

