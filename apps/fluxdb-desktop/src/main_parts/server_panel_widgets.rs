// 服务器面板共用的轻量控件与展示格式化。

fn server_toolbar_button(
    id: &'static str,
    icon: AppIcon,
    tooltip: String,
    disabled: bool,
    colors: UiColors,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .size(px(30.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .when(disabled, |this| this.opacity(0.55))
        .when(!disabled, |this| {
            this.cursor_pointer().hover(move |style| style.bg(colors.hover))
        })
        .flex()
        .items_center()
        .justify_center()
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(app_icon(
            icon,
            15.,
            if disabled { colors.muted } else { colors.text },
        ))
}

fn server_toggle(
    id: impl Into<gpui::ElementId>,
    label: String,
    active: bool,
    colors: UiColors,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(30.))
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(if active { colors.info } else { colors.border })
        .bg(if active { colors.tree_selected } else { colors.input_bg })
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .flex()
        .items_center()
        .gap_1()
        .text_size(ui_text_size(12.))
        .text_color(if active { colors.text } else { colors.muted })
        .when(active, |this| {
            this.child(app_icon(AppIcon::Check, 12., colors.info))
        })
        .child(label)
}

fn server_small_button(
    id: &'static str,
    label: String,
    icon: AppIcon,
    enabled: bool,
    danger: bool,
    colors: UiColors,
) -> Stateful<Div> {
    let fg = if !enabled {
        colors.muted
    } else if danger {
        colors.danger
    } else {
        colors.text
    };
    div()
        .id(id)
        .debug_selector(move || id.into())
        .h(px(28.))
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .when(!enabled, |this| this.opacity(0.5))
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |style| style.bg(colors.hover))
        })
        .flex()
        .items_center()
        .gap_1()
        .text_size(ui_text_size(12.))
        .text_color(fg)
        .child(app_icon(icon, 13., fg))
        .child(label)
}

fn server_dialog_button(
    label: String,
    primary: bool,
    enabled: bool,
    colors: UiColors,
) -> Stateful<Div> {
    div()
        .id(gpui::ElementId::Name(
            format!("server-dialog-button-{}", label).into(),
        ))
        .debug_selector({
            let label = label.clone();
            move || format!("server-dialog-button-{label}")
        })
        .h(px(30.))
        .px_3()
        .rounded(colors.radius)
        .border_1()
        .border_color(if primary { colors.danger } else { colors.border })
        .bg(if primary { colors.danger } else { colors.input_bg })
        .when(!enabled, |this| this.opacity(0.45))
        .when(enabled, |this| this.cursor_pointer())
        .flex()
        .items_center()
        .justify_center()
        .text_size(ui_text_size(12.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(if primary {
            gpui::white()
        } else {
            colors.text.into()
        })
        .child(label)
}

fn server_check_row(label: String, checked: bool, colors: UiColors) -> Stateful<Div> {
    div()
        .id("server-panel-kill-ack")
        .debug_selector(|| "server-panel-kill-ack".into())
        .h(px(34.))
        .px_2()
        .rounded(colors.radius)
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .size(px(16.))
                .rounded(px(3.))
                .border_1()
                .border_color(if checked { colors.info } else { colors.border })
                .bg(if checked { colors.info } else { colors.input_bg })
                .flex()
                .items_center()
                .justify_center()
                .when(checked, |this| {
                    this.child(app_icon(AppIcon::Check, 11., gpui::white().into()))
                }),
        )
        .child(
            div()
                .text_size(ui_text_size(12.))
                .text_color(colors.text)
                .child(label),
        )
}

fn server_badge(label: String, color: gpui::Rgba, colors: UiColors) -> Div {
    div()
        .h(px(22.))
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(color)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .text_size(ui_text_size(11.))
        .text_color(color)
        .child(label)
}

fn server_count_badge(label: String, colors: UiColors) -> Div {
    div()
        .h(px(22.))
        .px_2()
        .rounded(colors.radius)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .text_size(ui_text_size(11.))
        .text_color(colors.muted)
        .child(label)
}

fn server_status_fact(label: String, value: String, colors: UiColors) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(label),
        )
        .child(
            div()
                .text_size(ui_text_size(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(value),
        )
}

fn server_detail_row(label: String, value: String, colors: UiColors) -> Div {
    div()
        .flex()
        .items_baseline()
        .gap_2()
        .child(
            div()
                .w(px(76.))
                .flex_none()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(ui_text_size(12.))
                .text_color(colors.text)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(value),
        )
}

fn server_sql_block(label: String, sql: &str, colors: UiColors) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(label),
        )
        .child(
            div()
                .max_h(px(150.))
                .overflow_y_scrollbar()
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .p_2()
                .text_size(ui_text_size(12.))
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_color(colors.text)
                .child(if sql.trim().is_empty() {
                    "-".to_string()
                } else {
                    sql.to_string()
                }),
        )
}

fn server_table_header(colors: UiColors, columns: Vec<(String, f32, bool)>) -> Div {
    div()
        .h(px(30.))
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .bg(colors.panel_alt)
        .border_b_1()
        .border_color(colors.border)
        .children(columns.into_iter().map(|(label, width, flex)| {
            server_cell(label, width, flex, colors.muted)
                .text_size(ui_text_size(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
        }))
}

fn server_cell(text: impl Into<SharedString>, width: f32, flex: bool, color: gpui::Rgba) -> Div {
    let cell = div()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(ui_text_size(12.))
        .text_color(color)
        .child(text.into());
    if flex {
        cell.flex_1().min_w(px(0.))
    } else {
        cell.w(px(width)).flex_none()
    }
}

fn server_empty(label: String, colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .items_center()
        .justify_center()
        .text_size(ui_text_size(12.))
        .text_color(colors.muted)
        .child(label)
}

fn server_inline_error(message: String, colors: UiColors) -> Div {
    div()
        .mb_2()
        .px_3()
        .py_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.warning)
        .text_size(ui_text_size(12.))
        .text_color(colors.warning)
        .child(message)
}

fn server_session_status(session: &ServerSession) -> String {
    let command = session.command.trim().to_ascii_lowercase();
    if command == "sleep" || command.starts_with("idle") {
        tr("app.server_panel.status.idle").to_string()
    } else if command == "wait" || !session.wait_event.trim().is_empty() {
        tr("app.server_panel.status.waiting").to_string()
    } else if command.is_empty() {
        tr("app.server_panel.status.unknown").to_string()
    } else {
        tr("app.server_panel.status.running").to_string()
    }
}

fn server_sql_preview(sql: &str, max_chars: usize) -> String {
    let compact = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut preview = compact.chars().take(max_chars).collect::<String>();
    preview.push('…');
    preview
}

/// 纯 `HH:MM:SS`，小时可超过 24，避免单位文案随语言变化。
fn server_format_secs(secs: u64) -> String {
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn server_or_unknown(value: &str) -> String {
    if value.trim().is_empty() {
        tr("app.server_panel.unknown").to_string()
    } else {
        value.to_string()
    }
}

/// 连接器返回原始锁类型（`RECORD` / `relation` 等），本地化只在展示层做。
fn server_lock_type_label(raw: &str) -> String {
    match raw.to_ascii_lowercase().as_str() {
        "record" | "row" | "tuple" | "key" | "rid" => {
            tr("app.server_panel.lock_type.row").to_string()
        }
        "table" | "relation" | "tm" | "object" => {
            tr("app.server_panel.lock_type.table").to_string()
        }
        "transactionid" | "virtualxid" | "tx" | "tid" => {
            tr("app.server_panel.lock_type.transaction").to_string()
        }
        "advisory" | "ul" => tr("app.server_panel.lock_type.advisory").to_string(),
        "page" => tr("app.server_panel.lock_type.page").to_string(),
        "metadata" => tr("app.server_panel.lock_type.metadata").to_string(),
        "" => "-".to_string(),
        other => other.to_string(),
    }
}

fn server_lock_state_label(raw: &str) -> String {
    if raw.eq_ignore_ascii_case("waiting") {
        tr("app.server_panel.status.waiting").to_string()
    } else if raw.trim().is_empty() {
        "-".to_string()
    } else {
        raw.to_string()
    }
}

fn server_metric_label(key: &str) -> String {
    let label = match key {
        "Threads_connected" => "app.server_panel.metric.threads_connected",
        "Threads_running" => "app.server_panel.metric.threads_running",
        "Innodb_row_lock_waits" => "app.server_panel.metric.innodb_row_lock_waits",
        "Slow_queries" => "app.server_panel.metric.slow_queries",
        "Queries" => "app.server_panel.metric.queries",
        "Max_used_connections" => "app.server_panel.metric.max_used_connections",
        "max_connections" => "app.server_panel.metric.max_connections",
        "numbackends" => "app.server_panel.metric.numbackends",
        "xact_commit" => "app.server_panel.metric.xact_commit",
        "blks_hit" => "app.server_panel.metric.blks_hit",
        "blks_read" => "app.server_panel.metric.blks_read",
        "deadlocks" => "app.server_panel.metric.deadlocks",
        "temp_bytes" => "app.server_panel.metric.temp_bytes",
        "tup_returned" => "app.server_panel.metric.tup_returned",
        "tup_fetched" => "app.server_panel.metric.tup_fetched",
        "active_queries" => "app.server_panel.metric.active_queries",
        "lock_waits" => "app.server_panel.metric.lock_waits",
        "sessions" => "app.server_panel.metric.sessions",
        "active_requests" => "app.server_panel.metric.active_requests",
        "active_sessions" => "app.server_panel.metric.active_sessions",
        "cpu_count" => "app.server_panel.metric.cpu_count",
        "physical_memory_kb" => "app.server_panel.metric.physical_memory_kb",
        "session_logical_reads" => "app.server_panel.metric.session_logical_reads",
        "physical_reads" => "app.server_panel.metric.physical_reads",
        "execute_count" => "app.server_panel.metric.execute_count",
        _ => return key.to_string(),
    };
    tr(label).to_string()
}
