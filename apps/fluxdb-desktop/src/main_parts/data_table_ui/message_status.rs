#[derive(Clone, Copy, Debug, PartialEq)]
struct AppMessageLayout {
    bottom: f32,
    max_width: f32,
}

fn app_message_layout_for_width(width: f32) -> AppMessageLayout {
    AppMessageLayout {
        bottom: 40.,
        max_width: (width - 96.).clamp(320., 920.),
    }
}

fn app_message_layout(window: &Window) -> AppMessageLayout {
    app_message_layout_for_width(f32::from(window.bounds().size.width))
}

fn app_message_overlay(message: &AppMessage, window: &Window, colors: UiColors) -> Div {
    let (bg, text_color) = app_message_colors(message.kind, colors);
    let layout = app_message_layout(window);
    div()
        .absolute()
        .bottom(px(layout.bottom))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            div()
                .max_w(px(layout.max_width))
                .min_h(px(28.))
                .px_4()
                .py_1()
                .rounded(colors.radius_lg)
                .bg(bg)
                .overflow_hidden()
                .shadow(vec![box_shadow(
                    px(0.),
                    px(4.),
                    px(12.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.18 } else { 0.12 }),
                )])
                .flex()
                .items_center()
                .justify_center()
                .text_center()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(text_color)
                .child(
                    div()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_normal()
                        .line_clamp(2)
                        .child(message.text.clone()),
                ),
        )
}

fn app_message_colors(kind: AppMessageKind, colors: UiColors) -> (gpui::Rgba, gpui::Rgba) {
    match kind {
        AppMessageKind::Info => {
            if colors.is_dark {
                (rgb(0x2b3038), rgb(0xe8eaed))
            } else {
                (rgb(0x20242a), rgb(0xffffff))
            }
        }
        AppMessageKind::Success => {
            if colors.is_dark {
                (rgb(0x2b3038), rgb(0xe8eaed))
            } else {
                (rgb(0x20242a), rgb(0xffffff))
            }
        }
        AppMessageKind::Warning => (rgb(0xfff2cc), rgb(0x5f3b00)),
        AppMessageKind::Error => (rgb(0xffe0e0), rgb(0x9f1d1d)),
    }
}

fn app_event_message(event: &AppEvent) -> Option<(String, AppMessageKind)> {
    match event {
        AppEvent::Failed(error) => Some((
            format!("{}：{}", error.title, error.message),
            AppMessageKind::Error,
        )),
        _ => None,
    }
}

fn cell_value_label(value: &CellValue) -> String {
    match value {
        CellValue::Null => "NULL".to_string(),
        CellValue::Bool(value) => value.to_string(),
        CellValue::I64(value) => value.to_string(),
        CellValue::F64(value) => value.to_string(),
        CellValue::Text(value) => value.clone(),
        CellValue::Bytes(value) => format!("(BLOB) {} bytes", value.len()),
        CellValue::BinarySummary(_) => value.display_label(),
        CellValue::Json(value) => value.clone(),
    }
}

fn data_cell_edit_text(value: &CellValue) -> String {
    match value {
        CellValue::Null => String::new(),
        _ => cell_value_label(value),
    }
}

fn data_cell_edit_text_unchanged(current: &CellValue, input: &str) -> bool {
    data_cell_edit_text(current) == input
}

fn cell_value_length(value: &CellValue) -> usize {
    match value {
        CellValue::Null => 0,
        CellValue::Bytes(value) => value.len(),
        CellValue::BinarySummary(summary) => summary.byte_length as usize,
        _ => cell_value_label(value).chars().count(),
    }
}

fn looks_like_json(value: &str) -> bool {
    let value = value.trim();
    (value.starts_with('{') && value.ends_with('}'))
        || (value.starts_with('[') && value.ends_with(']'))
}

fn format_json_text(value: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(value)
        .ok()
        .and_then(|json| serde_json::to_string_pretty(&json).ok())
}

fn statusbar(
    state: &AppState,
    sql_file_tasks: &[SqlFileExecutionTaskState],
    data_export_tasks: &[TableDataExportTaskState],
    backup_tasks: &[BackupTaskState],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .h(px(24.))
        .bg(colors.status_bg)
        .border_t_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .text_size(px(14.))
        .text_color(colors.text)
        .child(div().w(px(260.)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .justify_center()
                .items_center()
                .child(statusbar_center(state, colors)),
        )
        .child(table_data_export_statusbar_area(data_export_tasks, colors, cx))
        .child(sql_file_statusbar_area(sql_file_tasks, colors, cx))
        .child(backup_statusbar_area(backup_tasks, colors, cx))
}

fn statusbar_summary(state: &AppState) -> String {
    let summary = format!("{} 个对象", visible_objects(state).len());

    statusbar_context(state)
        .map(|context| format!("{context} · {summary}"))
        .unwrap_or(summary)
}

/// 底栏中部概要整体：上下文 + 对象/连接数文本，右侧并列 Redis 连接概览指标
/// （版本 / 内存 / CPU），指标带悬停说明 tooltip。
fn statusbar_center(state: &AppState, colors: UiColors) -> impl IntoElement {
    let mut group = div()
        .flex()
        .items_center()
        .gap_3()
        .text_size(px(14.))
        .child(statusbar_summary(state));
    if let Some(metrics) = statusbar_metrics_element(state, colors) {
        group = group.child(metrics);
    }
    group
}

/// Redis 连接概览指标组（版本 / 内存 / CPU）。
/// 读取当前激活标签页对应连接的 `redis_overview`，概览尚未加载（或非 Redis）时返回 None。
fn statusbar_metrics_element(state: &AppState, colors: UiColors) -> Option<Div> {
    let connection_id = statusbar_active_connection_id(state)?;
    let overview: RedisConnectionOverview = state
        .connections
        .iter()
        .find(|connection| connection.config.id == connection_id)?
        .redis_overview
        .clone();
    if overview.version.is_empty()
        && overview.used_memory_bytes == 0
        && overview.cpu_usage_percent.is_none()
    {
        // 概览尚未加载或该连接非 Redis，跳过指标
        return None;
    }

    let mut row = div()
        .flex()
        .items_center()
        .border_l_1()
        .border_color(colors.border)
        .pl_3();
    if !overview.version.is_empty() {
        row = row.child(statusbar_metric(
            colors,
            "Redis 服务端版本",
            "版本",
            &overview.version,
        ));
    }
    if overview.used_memory_bytes > 0 {
        let memory = format_memory_bytes(overview.used_memory_bytes);
        row = row.child(statusbar_metric(colors, "Redis 实例内存占用", "内存", &memory));
    }
    if let Some(cpu) = overview.cpu_usage_percent {
        let cpu_text = format!("{cpu:.2}%");
        row = row.child(statusbar_metric(
            colors,
            "Redis 进程 CPU 使用率",
            "CPU",
            &cpu_text,
        ));
    }
    Some(row)
}

/// 单个概览指标悬停提示：hover 高亮背景 + 说明 tooltip。
fn statusbar_metric(
    colors: UiColors,
    tooltip: &'static str,
    label: &str,
    value: &str,
) -> Stateful<Div> {
    div()
        .id(tooltip)
        .h(px(18.))
        .rounded(colors.radius * 0.5)
        .px_1()
        .flex()
        .items_center()
        .text_size(px(13.))
        .text_color(colors.text)
        .hover(move |style| style.bg(colors.hover))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(format!("{label} {value}"))
}

/// 当前激活标签页对应的连接 ID（与概要上下文一致），用于定位 Redis 概览指标。
fn statusbar_active_connection_id(state: &AppState) -> Option<ConnectionId> {
    let tab = state.active_tab()?;
    match &tab.kind {
        TabKind::DataEditor(editor) => Some(editor.object.connection_id),
        TabKind::ObjectList(list) => list.parent.as_ref().map(|path| path.connection_id),
        TabKind::QueryEditor(editor) => Some(editor.connection_id),
        TabKind::RedisWorkbench(workbench) => Some(workbench.connection_id),
        TabKind::RedisCli(cli) => Some(cli.connection_id),
        TabKind::RedisPubSub(pubsub) => Some(pubsub.connection_id),
        TabKind::CreateTable(create) => Some(create.connection_id),
        TabKind::UserAdmin(admin) => Some(admin.connection_id),
        TabKind::BackupList(list) => Some(list.connection_id),
        TabKind::Settings => None,
    }
}

/// 把字节数格式化为可读容量字符串（B / KB / MB / GB），整数值去掉多余小数位。
fn format_memory_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = UNITS[0];
    for next in UNITS.iter().skip(1) {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = next;
    }
    if unit == "B" {
        format!("{value:.0} {unit}")
    } else {
        format!("{value:.1} {unit}").replace(".0 ", " ")
    }
}

fn statusbar_context(state: &AppState) -> Option<String> {
    let tab = state.active_tab()?;
    match &tab.kind {
        TabKind::DataEditor(editor) => object_path_statusbar_context(state, &editor.object, true),
        TabKind::ObjectList(list) => list
            .parent
            .as_ref()
            .and_then(|path| object_path_statusbar_context(state, path, false)),
        TabKind::QueryEditor(editor) => connection_statusbar_context(
            state,
            editor.connection_id,
            editor.database.as_deref(),
            None,
        ),
        TabKind::RedisWorkbench(workbench) => connection_statusbar_context(
            state,
            workbench.connection_id,
            Some(&workbench.database.to_string()),
            Some("Redis 命令"),
        ),
        TabKind::RedisCli(cli) => connection_statusbar_context(
            state,
            cli.connection_id,
            Some(&cli.database.to_string()),
            Some("Redis CLI"),
        ),
        TabKind::RedisPubSub(pubsub) => connection_statusbar_context(
            state,
            pubsub.connection_id,
            Some(&pubsub.database.to_string()),
            Some("Redis Pub/Sub"),
        ),
        TabKind::CreateTable(create) => connection_statusbar_context(
            state,
            create.connection_id,
            create.database.as_deref(),
            Some(if create.is_design() { "设计表" } else { "新建表" }),
        ),

        TabKind::UserAdmin(admin) => connection_statusbar_context(
            state,
            admin.connection_id,
            Some("用户与权限"),
            None,
        ),
        TabKind::BackupList(list) => connection_statusbar_context(
            state,
            list.connection_id,
            Some(&list.database),
            Some("备份列表"),
        ),
        TabKind::Settings => None,
    }
}

fn object_path_statusbar_context(
    state: &AppState,
    path: &ObjectPath,
    include_object: bool,
) -> Option<String> {
    connection_statusbar_context(
        state,
        path.connection_id,
        path.database.as_deref(),
        include_object.then_some(path.name.as_str()),
    )
}

fn connection_statusbar_context(
    state: &AppState,
    connection_id: ConnectionId,
    database: Option<&str>,
    object: Option<&str>,
) -> Option<String> {
    let connection = state
        .connections
        .iter()
        .find(|connection| connection.config.id == connection_id)?;
    let mut parts = vec![connection.config.name.as_str()];
    if let Some(database) = database.filter(|database| !database.is_empty()) {
        parts.push(database);
    }
    if let Some(object) = object.filter(|object| !object.is_empty()) {
        parts.push(object);
    }
    Some(parts.join(" / "))
}

fn table_icon(size: f32, colors: UiColors) -> impl IntoElement {
    div()
        .size(px(size))
        .rounded(colors.radius_lg)
        .bg(rgb(0xa8ddfb))
        .border_t_8()
        .border_color(rgb(0x118ee9))
        .flex()
        .items_center()
        .justify_center()
        .child(app_icon(AppIcon::Table, size * 0.58, rgb(0xeaf7ff)))
}
