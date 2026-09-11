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

/// `Alert` 非 banner 模式把图标塞进一个固定 `mt(5px)` 的壳里，图标尺寸又跟随字号
/// （`Alert` 用 `text_sm` = 0.875rem）。那个 5px 是按 gpui 的默认行高（φ × 字号 ≈ 22.7px）
/// 对的中线；我们把行高压紧之后，图标就会明显偏下，需要按中线对齐反推一个负 margin 补回去。
const ALERT_ICON_WRAPPER_MARGIN: f32 = 5.;
/// `Alert` 正文与图标的字号相对根字号的比值（`text_sm` = 0.875rem）。
const ALERT_TEXT_SM_RATIO: f32 = 0.875;

/// 提示类型 → Alert 变体 / 内置图标 / 主题语义色，一次收敛，避免三处各写一遍 match。
///
/// 四个语义色应用都没有覆盖，用的是 gpui-component 主题（默认即 shadcn neutral）的取值。
fn app_message_style(kind: AppMessageKind, cx: &App) -> (AlertVariant, IconName, gpui::Hsla) {
    let theme = ComponentTheme::global(cx);
    match kind {
        AppMessageKind::Info => (AlertVariant::Info, IconName::Info, theme.info),
        AppMessageKind::Success => (AlertVariant::Success, IconName::CircleCheck, theme.success),
        AppMessageKind::Warning => (AlertVariant::Warning, IconName::TriangleAlert, theme.warning),
        AppMessageKind::Error => (AlertVariant::Error, IconName::CircleX, theme.danger),
    }
}

/// 按当前行高把 `Alert` 的图标拉回文字中线，原因见 `ALERT_ICON_WRAPPER_MARGIN`。
///
/// 这里写全路径 `gpui_component::Icon`：根作用域里的 `Icon` 已被托盘图标（`tray_icon::Icon`）占用。
fn alert_icon(name: IconName, cx: &App, line_height: f32) -> gpui_component::Icon {
    let font_size = f32::from(ComponentTheme::global(cx).font_size) * ALERT_TEXT_SM_RATIO;
    let offset = ALERT_ICON_WRAPPER_MARGIN + font_size / 2. - line_height / 2.;
    gpui_component::Icon::new(name).mt(px(-offset))
}

/// 告警淡色底：把语义色按 `ALERT_TINT` 比例混进面板底色，得到**不透明**的淡色块。
///
/// 不直接用 `Alert` 自带的底色：它是 `语义色.mix_oklab(transparent_white(), 0.04)`，
/// 而 `mix_oklab` 连 alpha 一起插值，结果 alpha 只有 0.04 —— 那是给「贴在面板上的内联提示」
/// 准备的，浮在内容之上时等于全透明，必须由调用方覆盖。
/// 文字色与描边仍走 `Alert` 的变体色（语义色），即「淡色底 + 同色文字 + 同色描边」。
fn alert_tint(accent: gpui::Hsla, colors: UiColors) -> gpui::Hsla {
    accent.mix_oklab(colors.panel_bg.into(), ALERT_TINT)
}

/// 底部提示浮层：挂在窗口最上层、贴底居中；3s 后由 `NavicatMain::show_message` 的计时任务清掉。
/// 视觉承载统一交给 gpui-component 的 `Alert`（图标与配色随变体走）。
fn app_message_overlay(message: &AppMessage, window: &Window, cx: &App, colors: UiColors) -> Div {
    let layout = app_message_layout(window);
    let (variant, icon_name, accent) = app_message_style(message.kind, cx);
    // 同一条提示只存在一个实例；id 随消息自增，避免复用上一条的交互态。
    let id = ("app-message", message.id);
    let alert = Alert::new(id, message.text.clone())
        .with_variant(variant)
        .icon(alert_icon(icon_name, cx, APP_MESSAGE_LINE_HEIGHT))
        .bg(alert_tint(accent, colors))
        .line_height(px(APP_MESSAGE_LINE_HEIGHT));

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
                .min_w(px(0.))
                .shadow(vec![box_shadow(
                    px(0.),
                    px(4.),
                    px(12.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.18 } else { 0.12 }),
                )])
                .child(alert),
        )
}

/// 页面级错误块：常驻的 gpui-component `Alert`（不参与自动消失），顶部对齐放在内容区，
/// 承载错误标题、原始报文与可选细节；`actions` 是调用方自备的操作区（重试 / 复制错误等）。
///
/// `Alert` 是整行块级布局且没有 actions 槽，所以操作区作为它的兄弟节点纵向排在下方；
/// 整块限宽，避免长报文在宽窗口里横向铺满、把布局撑变形。
fn page_error_alert(
    id: gpui::ElementId,
    title: &str,
    message: &str,
    detail: Option<&str>,
    actions: Option<gpui::AnyElement>,
    cx: &App,
    colors: UiColors,
) -> Div {
    let (variant, icon_name, accent) = app_message_style(AppMessageKind::Error, cx);
    let mut column = div()
        .max_w(px(880.))
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap_3()
        .child(
            Alert::new(id, message.to_string())
                .with_variant(variant)
                .icon(alert_icon(icon_name, cx, PAGE_ERROR_LINE_HEIGHT))
                .title(title.to_string())
                .bg(alert_tint(accent, colors))
                .line_height(px(PAGE_ERROR_LINE_HEIGHT)),
        );

    if let Some(detail) = detail.map(str::trim).filter(|detail| !detail.is_empty()) {
        column = column.child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(detail.to_string()),
        );
    }
    if let Some(actions) = actions {
        column = column.child(div().flex().items_center().gap_2().child(actions));
    }
    column
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
