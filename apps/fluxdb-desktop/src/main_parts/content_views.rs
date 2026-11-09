// 结果区高度模型（上下分栏）：最小 = 编辑器高 × `RESULT_MIN_EDITOR_RATIO`（结果最小仍有意义），
// 最大 = 分栏高 × 各自的 `_MAX_SPLIT_RATIO`（结果可拉得较大，但为编辑器保留空间）。
// min：result = 0.20·(split−result) ⇒ split/6。
const RESULT_MIN_EDITOR_RATIO: f32 = 0.20;
// SQL 结果区最大 = 分栏 × 0.70（编辑器至少留 30%）；Workbench 结果区最大 = 分栏 × 0.80。
const QUERY_RESULT_MAX_SPLIT_RATIO: f32 = 0.70;
const REDIS_WB_RESULT_MAX_SPLIT_RATIO: f32 = 0.80;
// 默认结果区高：split×0.375（≈ min 与 max 之间，适中）。
const RESULT_DEFAULT_SPLIT_RATIO: f32 = 0.375;
const QUERY_OUTPUT_DEFAULT_WIDTH: f32 = 560.;
const QUERY_OUTPUT_MIN_WIDTH: f32 = 320.;
const SQL_EDITOR_MIN_WIDTH: f32 = 360.;
const QUERY_TOOLBAR_HEIGHT: f32 = 40.;
// Redis Workbench 分栏：编辑器区与结果区可拖动调整大小（同一结果区模型）。
// 「下方」布局调整高度，「右侧」布局调整宽度，两种布局各自记住自己的尺寸。
const REDIS_WB_TOOLBAR_HEIGHT: f32 = 40.;
// 右侧布局下编辑器区宽度的夹取边界：编辑器不至窄到没法写命令，结果区也始终留得下。
const REDIS_WB_EDITOR_MIN_WIDTH: f32 = 320.;
const REDIS_WB_RESULT_MIN_WIDTH: f32 = 320.;
// 上下分栏的像素下限：占比下限在窗口较矮时仍可能把一侧压到几十像素，
// 这里再兜一层绝对高度，保证编辑器能看清光标、结果区能看出一行记录。
const WORKBENCH_EDITOR_MIN_HEIGHT: f32 = 100.;
const WORKBENCH_RESULT_MIN_HEIGHT: f32 = 120.;
// 单条执行记录卡片的结果区最大高度：超长 reply 在卡片内滚动，避免把整张卡片 / 外层列表撑高。
const REDIS_WB_RECORD_BODY_MAX_HEIGHT: f32 = 320.;
// 执行摘要的数值列按完整表头留宽；空间不足时由整张表横向滚动，而不是压缩成省略号。
const QUERY_SUMMARY_STATEMENT_MIN_WIDTH: f32 = 280.;
const QUERY_SUMMARY_TYPE_WIDTH: f32 = 240.;
const QUERY_SUMMARY_RETURNED_WIDTH: f32 = 72.;
const QUERY_SUMMARY_AFFECTED_WIDTH: f32 = 72.;
const QUERY_SUMMARY_ELAPSED_WIDTH: f32 = 76.;
const QUERY_SUMMARY_TABLE_MIN_WIDTH: f32 = QUERY_SUMMARY_STATEMENT_MIN_WIDTH
    + QUERY_SUMMARY_TYPE_WIDTH
    + QUERY_SUMMARY_RETURNED_WIDTH
    + QUERY_SUMMARY_AFFECTED_WIDTH
    + QUERY_SUMMARY_ELAPSED_WIDTH;

/// 垂直分栏（编辑器在结果区上方）可用高度估算：视口高度扣除顶部工具条。
fn redis_workbench_split_height(window: &Window) -> f32 {
    (f32::from(window.viewport_size().height) - REDIS_WB_TOOLBAR_HEIGHT).max(0.)
}

/// 水平分栏（编辑器在结果区左侧）可用宽度估算：视口宽度扣除连接侧边栏。
///
/// 注意必须减掉侧边栏：它宽达 180-600px，在 1400px 窗口里最多占 43%。
/// 若按视口宽直接算，渲染与实际可用宽会差出一整条侧边栏，
/// 拖动时面板会每帧跳。收起侧边栏时不计入。
fn redis_workbench_split_width(this: &NavicatMain, window: &Window) -> f32 {
    let sidebar = if this.show_connection_browser {
        clamp_connection_browser_width(this.connection_browser_width)
    } else {
        0.
    };
    (f32::from(window.viewport_size().width) - sidebar).max(0.)
}

/// 结果区高度（像素）夹到 [split/6, 0.70·split]（SQL）。
/// min：result=0.20·editor ⇒ split/6；max：直接占分栏 `QUERY_RESULT_MAX_SPLIT_RATIO`。
fn query_output_clamp_height(result_height: f32, split_height: f32) -> f32 {
    let min_result = split_height / (1. + 1. / RESULT_MIN_EDITOR_RATIO);
    let max_result = split_height * QUERY_RESULT_MAX_SPLIT_RATIO;
    if min_result >= max_result {
        split_height / 4.
    } else {
        result_height.clamp(min_result, max_result)
    }
}

/// 把上下分栏的编辑器高度（像素）夹到「结果区 ∈ [split/6, 0.80·split]」对应的范围。
/// 结果最小 ⇒ editor 最大 = split − split/6 = 5·split/6；结果最大(0.80·split) ⇒ editor 最小 = 0.20·split。
/// 占比下限之外再叠加像素下限，避免矮窗口把某一侧压到只剩几十像素。
/// 可用空间过小时退化为对半，避免 `clamp` 因 min>max 触发 panic。
/// 只用于高度：宽度的像素下限不同，见 `redis_workbench_editor_width`。
fn workbench_clamp_editor_size(size: f32, split: f32) -> f32 {
    let min_editor = (split * (1. - REDIS_WB_RESULT_MAX_SPLIT_RATIO))
        .max(WORKBENCH_EDITOR_MIN_HEIGHT);
    let max_editor = (split * (1. - 1. / (1. + 1. / RESULT_MIN_EDITOR_RATIO)))
        .min(split - WORKBENCH_RESULT_MIN_HEIGHT);
    if min_editor >= max_editor {
        split / 2.
    } else {
        size.clamp(min_editor, max_editor)
    }
}

/// 由全局分栏占比换算编辑器区高度（像素）。
fn workbench_editor_height(ratio: f32, split_height: f32) -> f32 {
    workbench_clamp_editor_size(split_height * ratio, split_height)
}

/// 右侧布局下编辑器区宽度（像素）：取自设置，夹到
/// `[REDIS_WB_EDITOR_MIN_WIDTH, available − REDIS_WB_RESULT_MIN_WIDTH]`。
/// 两种布局的尺寸来源刻意不同 —— 高度是持久化占比，宽度是持久化像素；
/// 宽度不用占比是因为它没有可信基准（见 `redis_workbench_split_width`）。
fn redis_workbench_editor_width(width: f32, available: f32) -> f32 {
    let max_width = (available - REDIS_WB_RESULT_MIN_WIDTH).max(REDIS_WB_EDITOR_MIN_WIDTH);
    width.clamp(REDIS_WB_EDITOR_MIN_WIDTH, max_width)
}

fn content(
    state: &AppState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    match state.active_tab() {
        Some(tab) if matches!(tab.kind, TabKind::AiChat { .. }) => {
            div().size_full().min_h(px(0.)).flex().flex_col()
                .child(ai_chat_panel(this, window, colors, cx))
        }
        Some(tab) if matches!(tab.kind, TabKind::DataEditor(_)) => {
            let TabKind::DataEditor(editor) = &tab.kind else {
                unreachable!();
            };
            data_editor_content(tab.id, editor, this, window, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::QueryEditor(_)) => {
            let TabKind::QueryEditor(editor) = &tab.kind else {
                unreachable!();
            };
            query_editor_content(tab.id, editor, this, window, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::RedisWorkbench(_)) => {
            let TabKind::RedisWorkbench(workbench) = &tab.kind else {
                unreachable!();
            };
            redis_workbench_content(tab.id, workbench, this, window, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::RedisCli(_)) => {
            let TabKind::RedisCli(cli) = &tab.kind else {
                unreachable!();
            };
            redis_cli_content(tab.id, cli, this, window, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::RedisPubSub(_)) => {
            let TabKind::RedisPubSub(pubsub) = &tab.kind else {
                unreachable!();
            };
            redis_pubsub_content(tab.id, pubsub, this, window, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::MongoCli(_)) => {
            let TabKind::MongoCli(cli) = &tab.kind else {
                unreachable!();
            };
            mongo_cli_content(tab.id, cli, this, window, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::MongoWorkbench(_)) => {
            let TabKind::MongoWorkbench(workbench) = &tab.kind else {
                unreachable!();
            };
            div().size_full().min_h(px(0.)).flex().flex_col().child(
                mongo_workbench_content(tab.id, workbench, this, window, colors, cx),
            )
        }
        Some(tab) if matches!(tab.kind, TabKind::MongoCollection(_)) => {
            let TabKind::MongoCollection(collection) = &tab.kind else {
                unreachable!();
            };
            div().size_full().min_h(px(0.)).flex().flex_col().child(
                this.mongo_collection_content(tab.id, collection, window, colors, cx),
            )
        }
        Some(tab) if matches!(tab.kind, TabKind::CreateTable(_)) => {
            let TabKind::CreateTable(create) = &tab.kind else {
                unreachable!();
            };
            create_table_content(tab.id, create, this, window, colors, cx)
        }

        Some(tab) if matches!(tab.kind, TabKind::ObjectList(_)) => {
            object_list_content(state, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::BackupList(_)) => {
            let TabKind::BackupList(list) = &tab.kind else {
                unreachable!();
            };
            backup_list_content(state, this, list, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::ErDiagram(_)) => {
            let TabKind::ErDiagram(er) = &tab.kind else {
                unreachable!();
            };
            er_diagram_content(tab.id, er, this, window, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::UserAdmin(_)) => {
            let TabKind::UserAdmin(admin) = &tab.kind else {
                unreachable!();
            };
            user_admin_content(state, tab.id, admin, this, window, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::Settings(_)) => {
            settings_content(
                state,
                &this.update_ui,
                AiSettingsPanelState {
                    filter: this.ai_model_filter.clone(),
                    filter_input: this.ai_model_filter_input.clone(),
                    testing_id: this.ai_model_testing_id.clone(),
                    deleting_id: this.ai_model_deleting_id.clone(),
                    settings_loading: this.ai_settings_loading,
                    settings_error: this.ai_settings_error.clone(),
                    session_filter_input: this.ai_session_filter_input.clone(),
                    session_filter: this.ai_session_filter.clone(),
                    session_archiving_id: this.ai_session_archiving_id.clone(),
                    tools_tab: this.ai_tools_tab,
                    tool_settings: this.controller.state().ai_tool_settings.clone(),
                    mcp_install_status: this.ai_mcp_install_status.clone(),
                    mcp_clients_busy: this.ai_mcp_clients_busy.clone(),
                    mcp_clients_error: this.ai_mcp_clients_error.clone(),
                    mcp_connection_details: this.ai_mcp_connection_details.clone(),
                    mcp_connection_error: this.ai_mcp_connection_error.clone(),
                    mcp_details_open: this.ai_mcp_details_open,
                    audit_filter: this.ai_audit_filter,
                    audit_filter_input: this.ai_audit_filter_input.clone(),
                    audit_filter_text: this.ai_audit_filter_text.clone(),
                    expanded_audit: this.ai_audit_expanded.clone(),
                    agent_editor: this.ai_agent_editor.clone(),
                },
                this.theme_mode,
                this.settings_panel_section,
                colors,
                this.settings_editor_draft.clone(),
                this.settings_font_size_slider.clone(),
                this.settings_line_height_input.clone(),
                this.settings_radius_input.clone(),
                [this.pg_client.snapshot(NativeClientKind::Postgres), this.mysql_client.snapshot(NativeClientKind::MySql)],
                this.settings_dangerous_actions_collapsed,
                this.settings_data_groups_collapsed,
                window,
                cx,
            )
        }
        _ => workbench_home(state, window, colors, cx),
    }
}

fn workbench_home(
    state: &AppState,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let connected_count = state
        .connections
        .iter()
        .filter(|connection| connection.connected)
        .count();
    let kind_count = database_kind_count(state);
    let width = window.viewport_size().width;
    let compact_home = width < px(1180.);
    let stat_cols = if width < px(880.) {
        1
    } else if width < px(1280.) {
        2
    } else {
        3
    };

    div()
        .flex_1()
        .bg(colors.content_bg)
        .border_r_1()
        .border_color(colors.border)
        .overflow_hidden()
        .child(
            div().size_full().p_6().flex().justify_center().child(
                div()
                    .w_full()
                    .max_w(px(1180.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .grid()
                            .grid_cols(stat_cols)
                            .gap_3()
                            .child(home_stat_card(
                                tr("app.home.stat.connections"),
                                state.connections.len(),
                                tr("app.home.stat.connections_hint"),
                                colors,
                            ))
                            .child(home_stat_card(
                                tr("app.home.stat.connected"),
                                connected_count,
                                tr("app.home.stat.connected_hint"),
                                colors,
                            ))
                            .child(home_stat_card(
                                tr("app.home.stat.database_kinds"),
                                kind_count,
                                tr("app.home.stat.database_kinds_hint"),
                                colors,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .when(compact_home, |this| this.flex_col())
                            .gap_4()
                            .child(recent_connections_panel(state, colors, cx).flex_1())
                            .child(
                                common_actions_panel(state, window, colors, cx)
                                    .when(!compact_home, |this| this.w(px(360.))),
                            ),
                    ),
            ),
        )
}

include!("settings/navigation.rs");
include!("settings/editor.rs");
include!("settings/appearance.rs");
include!("settings/system.rs");
include!("settings/native_client.rs");
include!("settings/rows.rs");
include!("settings/ai.rs");
include!("settings/preferences.rs");

fn home_stat_card(
    label: impl Into<SharedString>,
    value: usize,
    hint: impl Into<SharedString>,
    colors: UiColors,
) -> Div {
    let label = label.into();
    let hint = hint.into();
    div()
        .h(px(92.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .p_4()
        .flex()
        .flex_col()
        .justify_between()
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(label),
        )
        .child(
            div()
                .flex()
                .items_end()
                .gap_2()
                .child(
                    div()
                        .text_size(px(30.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(value.to_string()),
                )
                .child(
                    div()
                        .pb_1()
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child(hint),
                ),
        )
}

fn recent_connections_panel(
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut list = div().flex().flex_col();
    for connection in state.connections.iter().take(5) {
        list = list.child(recent_connection_row(connection, colors, cx));
    }

    if state.connections.is_empty() {
        list = list.child(
            div()
                .h(px(96.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(14.))
                .text_color(colors.muted)
                .child(tr("app.home.no_connections")),
        );
    }

    home_panel(tr("app.home.quick_start"), list, colors)
}

fn recent_connection_row(
    connection: &ConnectionState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let connection_id = connection.config.id;
    let color = connection_config_color(&connection.config.options);
    div()
        .h(px(68.))
        .border_b_1()
        .border_color(colors.border_soft)
        .px_4()
        .flex()
        .items_center()
        .justify_between()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                // 建连走侧边栏同款后台路径：不卡主线程，重复点击由 connecting_connections 去重。
                this.open_connection_from_sidebar(connection_id, cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(database_kind_badge(connection.config.kind, colors))
                .child(div().w(px(4.)).h(px(28.)).rounded_full().bg(color))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(connection.config.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(colors.muted)
                                .child(connection_summary(&connection.config)),
                        ),
                ),
        )
        .child(
            div()
                .text_size(px(18.))
                .text_color(if connection.connected {
                    rgb(0x16a34a)
                } else {
                    rgb(0xa0a7b2)
                })
                .child(if connection.connected { "●" } else { "○" }),
        )
}

fn common_actions_panel(
    state: &AppState,
    _window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let has_connection = !state.connections.is_empty();
    home_panel(
        tr("app.home.common_actions"),
        div()
            .flex()
            .flex_col()
            .p_2()
            .gap_1()
            .child(home_action_row("home-new-connection", tr("app.home.new_connection"), "＋", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.show_new_connection(window, cx);
                    cx.stop_propagation();
                }),
            ))
            .child(home_action_row("home-new-query", tr("app.home.new_query"), "SQL", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if has_connection {
                        this.open_new_query(None, cx);
                    }
                    cx.stop_propagation();
                }),
            ))
            .child(home_action_row("home-refresh-connections", tr("app.home.refresh_connections"), "⟳", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.refresh_connection_tree(cx);
                    cx.stop_propagation();
                }),
            ))
            .child(
                home_icon_action_row(tr("app.home.settings"), AppIcon::Settings, colors)
                    .id("home-settings")
                    .debug_selector(|| "home-settings".to_string())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.dispatch(AppCommand::OpenSettings, cx);
                            cx.stop_propagation();
                        }),
                    ),
            )
            .child(
                div()
                    .mt_2()
                    .rounded(colors.radius_lg)
                    .bg(colors.panel_alt)
                    .px_3()
                    .py_3()
                    .text_size(px(13.))
                    .text_color(colors.muted)
                    .child(tr("app.home.hint")),
            ),
        colors,
    )
}

fn home_panel(title: impl Into<SharedString>, body: Div, colors: UiColors) -> Div {
    let title = title.into();
    div()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .overflow_hidden()
        .child(
            div()
                .h(px(44.))
                .border_b_1()
                .border_color(colors.border_soft)
                .px_4()
                .flex()
                .items_center()
                .text_size(px(15.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(title),
        )
        .child(body)
}

fn home_icon_action_row(
    label: impl Into<SharedString>,
    icon: AppIcon,
    colors: UiColors,
) -> Div {
    let label = label.into();
    div()
        .h(px(44.))
        .rounded(colors.radius)
        .px_3()
        .flex()
        .items_center()
        .justify_between()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .text_size(px(14.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(label),
        )
        .child(app_icon_box(icon, 28., 15., colors.muted))
}

/// 首页快捷操作行。
///
/// `id` 是稳定业务标识（如 `home-new-connection`），供 UI 自动化与无障碍树定位；
/// 该行由调用方挂 `on_mouse_down`，是可交互元素，因此纳入 headless 观察。
fn home_action_row(
    id: &'static str,
    label: impl Into<SharedString>,
    glyph: &'static str,
    colors: UiColors,
) -> Stateful<Div> {
    let label = label.into();
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .aria_label(label.clone())
        .cursor_pointer()
        .h(px(44.))
        .rounded(colors.radius_lg)
        .px_3()
        .flex()
        .items_center()
        .gap_3()
        .text_size(px(14.))
        .text_color(colors.text)
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .w(px(30.))
                .h(px(24.))
                .rounded(colors.radius_lg)
                .bg(colors.panel_alt)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(glyph),
        )
        .child(label)
}

fn database_kind_badge(kind: DatabaseKind, colors: UiColors) -> impl IntoElement {
    div()
        .size(px(30.))
        .rounded(colors.radius_lg)
        .bg(rgb(0x24272d))
        .flex()
        .items_center()
        .justify_center()
        .child(img(database_kind_icon_path(kind)).size(px(20.)))
}

fn object_list_content(state: &AppState, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    let mut content = div()
        .flex_1()
        .bg(colors.panel_bg)
        .border_r_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .child(object_toolbar(colors))
        .child(table_header(colors));

    for object in visible_objects(state) {
        content = content.child(object_row(object, colors, cx));
    }

    content
}

fn data_editor_content(
    tab_id: TabId,
    editor: &DataEditorState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let is_redis_editor = matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey);
    let show_table_tools = !is_redis_editor;
    let filter_open = show_table_tools && this.data_filter_panels.contains(&tab_id);
    let search_open = show_table_tools && this.data_search_panels.contains(&tab_id);
    let field_filter_open = show_table_tools && this.field_filter_popover == Some(tab_id);
    if filter_open {
        this.sync_data_filter_text_inputs(tab_id, window, cx);
    }
    if search_open {
        let expected = this
            .data_search_queries
            .get(&tab_id)
            .cloned()
            .unwrap_or_default();
        let input_focused = this
            .data_search_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if !input_focused && this.data_search_input.read(cx).value().to_string() != expected {
            this.data_search_input.update(cx, |input, cx| {
                input.set_value(expected, window, cx);
            });
        }
    }
    let filter_rules = this
        .data_filter_draft_rules
        .get(&tab_id)
        .cloned()
        .unwrap_or_default();
    let sort_rules = this
        .data_sort_draft_rules
        .get(&tab_id)
        .cloned()
        .unwrap_or_default();
    let filter_popover = this.data_filter_popover;
    let field_filter_search = this.field_filter_search.clone();
    let field_filter_search_input = this.field_filter_search_input.clone();
    let value_search = this.data_filter_value_search.clone();
    let value_search_loading = this.data_filter_value_search_loading_until.is_some();
    let data_filter_value_input = this.data_filter_value_input.clone();
    let data_filter_batch_input = this.data_filter_batch_input.clone();
    let local_filter_value_input = this.local_filter_value_input.clone();
    let local_filter_search_input = this.local_filter_search_input.clone();
    let local_filter_popover = this.local_filter_popover.clone();
    let local_filter_manager_open = this.local_filter_manager_popover == Some(tab_id);
    let local_filter_manager_field = this.local_filter_manager_field.clone();
    let local_filter_manager_draft_filters = this.local_filter_manager_draft_filters.clone();
    let local_filter_manager_field_open = this.local_filter_manager_field_open;
    let local_filter_manager_values_open = this.local_filter_manager_values_open;
    let local_filter_value = this.local_filter_value.clone();
    let local_filter_search = this.local_filter_search.clone();
    let local_filter_draft_values = this.local_filter_draft_values.clone();
    let data_filter_search_input = this.data_filter_search_input.clone();
    let data_filter_text_input = this.data_filter_text_input.clone();
    let data_sql_panel_input = this.data_sql_panel_input.clone();
    let data_page_input = this.data_page_input.clone();
    let data_search_input = this.data_search_input.clone();
    let data_sql_footer_selection = this.data_sql_footer_selection.clone();
    let filter_applying = this.data_filter_applying_tabs.contains(&tab_id);
    let filter_mode = this
        .data_filter_modes
        .get(&tab_id)
        .copied()
        .unwrap_or(DataFilterMode::Builder);
    let available_data_height =
        (f32::from(window.viewport_size().height) - 40. - 24.).max(0.);
    let default_filter_panel_height =
        data_filter_panel_height(filter_rules.as_slice(), filter_mode, available_data_height);
    let filter_panel_height = this
        .data_filter_panel_heights
        .get(&tab_id)
        .copied()
        .map(|height| clamp_data_filter_panel_height(height, available_data_height))
        .unwrap_or(default_filter_panel_height);
    let filter_panel_max_height =
        (available_data_height * DATA_FILTER_PANEL_MAX_DATA_RATIO)
            .clamp(DATA_FILTER_PANEL_MIN_HEIGHT, DATA_FILTER_PANEL_MAX_HEIGHT)
            .min(available_data_height);
    let filter_text = this
        .data_filter_texts
        .get(&tab_id)
        .cloned()
        .unwrap_or_default();
    let sort_text = this
        .data_sort_texts
        .get(&tab_id)
        .cloned()
        .unwrap_or_default();
    let content = div()
        .relative()
        .flex_1()
        .bg(colors.panel_bg)
        .border_r_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .when(show_table_tools, |builder| {
            builder.child(data_editor_toolbar(
                tab_id,
                filter_open,
                this.data_filter_modes
                    .get(&tab_id)
                    .copied()
                    .unwrap_or(DataFilterMode::Builder),
                this.connection_database_kind(editor.object.connection_id),
                search_open,
                field_filter_open,
                None,
                None,
                show_table_tools,
                colors,
                cx,
            ))
        })
        .when(filter_open, |builder| builder);

    // 只有「首次加载、还没有任何数据」才整块替换成居中加载态。
    // 已有数据时（重新应用筛选/排序、翻页、刷新）保留旧表格，只由表格区域盖遮罩，
    // 避免筛选面板、SQL 预览和列头在等待期间整块消失。
    if editor.loading && editor.page.is_none() {
        return content.child(
            center_loading_message(tr("app.data.loading"), colors)
                .id(("data-page-loading", tab_id.0))
                .role(Role::Status)
                .aria_label(tr("app.data.loading"))
        );
    }

    let Some(page) = &editor.page else {
        if let Some(error) = &editor.error {
            // 加载失败：常驻 Alert + 重试。重试复用工具栏「刷新」的既有入口，
            // 按当前筛选/排序在后台重新拉取，不会阻塞 UI；也不再是盖住整个表格区的居中文本。
            let retry = Button::new(("retry-data-page", tab_id.0))
                .label(tr("app.common.retry"))
                .small()
                .outline()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.request_data_editor_refresh(tab_id, cx);
                }))
                .into_any_element();
            return content.child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scrollbar()
                    .p_6()
                    .child(page_error_alert(
                        gpui::ElementId::Name(
                            format!("data-editor-error-{}", tab_id.0).into(),
                        ),
                        &error.title,
                        &error.message,
                        error.detail.as_deref(),
                        Some(retry),
                        cx,
                        colors,
                    )),
            );
        }
        return content.child(center_message(tr("app.data.empty"), colors));
    };
    if is_redis_editor {
        this.sync_redis_search_controls(tab_id, window, cx);
    }
    // 先把 Redis 搜索/类型筛选后的展示页算出来，喂给下方列表与表格（与既有旧版一致）；
    // 非 Redis 编辑器则原样借用，不影响其它数据页。
    let display_page = this.data_page_for_display(tab_id, page);
    let page = display_page.as_ref();
    let db_kind = this.connection_database_kind(editor.object.connection_id);
    let sql = data_editor_sql_preview(
        &editor.object,
        &page.columns,
        filter_rules.as_slice(),
        sort_rules.as_slice(),
        filter_mode,
        filter_text.as_str(),
        sort_text.as_str(),
        page.offset,
        page.limit,
        db_kind,
    );
    let all_field_names = page
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let visible_field_names = this.visible_fields_for_page(tab_id, page);
    let field_count = Some((visible_field_names.len(), all_field_names.len()));
    let table_state = this.data_table_state(tab_id, page, window, cx);
    let (search_matches, active_search_match, selected_source_row) = {
        let table = table_state.read(cx);
        let delegate = table.delegate();
        (
            delegate.search_matches.clone(),
            delegate.active_search_match,
            delegate
                .selected_row
                .or_else(|| delegate.selected_cell.map(|(row, _)| row))
                .and_then(|row| delegate.source_row_indexes.get(row).copied()),
        )
    };
    let search_highlight_all = this.data_search_highlight_all_tabs.contains(&tab_id);
    let change_count = editor
        .changes
        .as_ref()
        .filter(|changes| !changes.is_empty())
        .map(data_change_item_count);
    let change_sql_preview = editor
        .changes
        .as_ref()
        .filter(|changes| !changes.is_empty())
        .map(|changes| {
            let db_kind = this.connection_database_kind(changes.object.connection_id);
            data_change_sql_preview(page, changes, db_kind)
        });
    let change_sql_preview_open = this.data_change_sql_preview_tabs.contains(&tab_id);
    // min_h(0)：flex 项默认最小尺寸为内容高度，缺这行时整列会被内容撑出窗口，
    // 下游（如 Redis Set 成员列表）拿到的永远是内容高度而非可用高度，滚动区因此永不溢出
    let content = div()
        .relative()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .border_r_1()
        .border_color(colors.border)
        .flex()
        .flex_col();
    let content = content.when(show_table_tools, |builder| {
        builder.child(data_editor_toolbar(
            tab_id,
            filter_open,
            filter_mode,
            db_kind,
            search_open,
            field_filter_open,
            field_count,
            Some(editor.table_info.open),
            show_table_tools,
            colors,
            cx,
        ))
    });
    let content = content.when(filter_open, |builder| {
        builder.child(data_filter_panel(
            tab_id,
            sql.clone(),
            data_sql_panel_input,
            page,
            filter_rules.as_slice(),
            sort_rules.as_slice(),
            this.data_sort_clear_undo.contains_key(&tab_id),
            filter_mode,
            filter_panel_height,
            filter_panel_max_height,
            filter_applying,
            data_filter_text_input,
            db_kind,
            colors,
            window,
            cx,
        ))
    });
    let redis_search_active = is_redis_editor
        && (!this.redis_search_query(tab_id).is_empty()
            || this.redis_type_filter(tab_id) != REDIS_TYPE_FILTER_ALL);
    let redis_refreshing = is_redis_editor
        && (filter_applying
            || this._data_filter_apply_tasks.contains_key(&tab_id.0)
            || this._data_load_tasks.contains_key(&tab_id.0));
    let history_open = this.redis_key_search_history_open
        && tab_id == this.active_redis_data_tab_id().unwrap_or(tab_id);
    let content = content.when(is_redis_editor, |builder| {
        let search_bar = redis_key_search_bar(
            tab_id,
            this.redis_type_select.clone(),
            this.redis_search_input.clone(),
            redis_search_active,
            redis_refreshing,
            colors,
            cx,
        );
        // 全屏遮罩层捕捉外部点击关闭下拉；渲染在搜索栏前（下层），避免遮挡搜索栏与下拉菜单的交互。
        builder
            .when(history_open, |builder| {
                builder.child(redis_key_search_history_occlude(colors, cx))
            })
            .child(search_bar)
    });
    if is_redis_editor {
        let refreshed_at = this.redis_data_refresh_times.get(&tab_id).copied();
        let mut with_view = content.child(redis_split_view(
            tab_id,
            page,
            &table_state,
            refreshed_at,
            redis_refreshing,
            colors,
            this,
            window,
            cx,
        ));
        // 搜索历史下拉挂到 Redis 内容顶层、`redis_split_view` 之后：gpui 无 z-index，兄弟节点按
        // 绘制顺序叠放（后者在上），表格为不透明背景会把前置于它的子节点盖住，故下拉必须后置才能浮于表格之上。
        if history_open {
            with_view = with_view.child(redis_key_search_history_overlay(
                tab_id,
                this.redis_key_search_history_for(tab_id),
                colors,
                cx,
            ));
        }
        // 「新增 Key」抽屉作为全屏遮罩挂到 Redis 内容顶层（对齐 RedisInsight AddKey）；
        // 仅当前标签页有打开中的建 Key 抽屉时挂载。壳内三段式：公共字段 + 当前类型子表单。
        let add_key_open = this
            .redis_add_key_drawer
            .as_ref()
            .is_some_and(|drawer| drawer.tab_id == tab_id);
        let add_key_applying = this.redis_add_key_applying;
        return with_view.when(add_key_open, |builder| {
            // 按当前选中的 Key 类型渲染对应的子表单组件（类型切换由控制器订阅重置字段）。
            let kind = this.redis_add_key_kind(cx);
            let active_form = match kind {
                RedisAddKeyKind::String => {
                    redis_add_key_string_form(&this.redis_add_key_string_input, colors)
                }
                RedisAddKeyKind::Json => {
                    redis_add_key_json_form(
                        &this.redis_add_key_json_editor,
                        add_key_applying,
                        colors,
                        cx,
                    )
                }
                RedisAddKeyKind::Hash => {
                    redis_add_key_hash_form(&this.redis_add_key_hash_rows, add_key_applying, colors, cx)
                }
                RedisAddKeyKind::ZSet => {
                    redis_add_key_zset_form(&this.redis_add_key_zset_rows, add_key_applying, colors, cx)
                }
                RedisAddKeyKind::Set => {
                    redis_add_key_set_form(&this.redis_add_key_set_rows, add_key_applying, colors, cx)
                }
                RedisAddKeyKind::List => redis_add_key_list_form(
                    &this.redis_add_key_list_rows,
                    this.redis_add_key_list_direction,
                    add_key_applying,
                    colors,
                    cx,
                ),
                RedisAddKeyKind::Stream => redis_add_key_stream_form(
                    &this.redis_add_key_stream_id_input,
                    &this.redis_add_key_stream_rows,
                    add_key_applying,
                    colors,
                    cx,
                ),
            };
            builder.child(redis_add_key_drawer(
                tab_id,
                &this.redis_add_key_type_select,
                &this.redis_add_key_name_input,
                &this.redis_add_key_ttl_input,
                active_form,
                this.redis_add_key_scroll.clone(),
                add_key_applying,
                colors,
                window,
                cx,
            ))
        });
    }

    let cell_detail_input = this.cell_detail_input.clone();
    let cell_detail_drawer_height = this
        .cell_detail_drawer_heights
        .get(&tab_id)
        .copied()
        .map(clamp_cell_detail_drawer_height)
        .unwrap_or(CELL_DETAIL_DRAWER_DEFAULT_HEIGHT);
    let large_value_window = editor.cell_detail_panel.open.then(|| {
        let cell = editor.cell_detail_panel.active_cell?;
        let large = page
            .rows
            .get(cell.row)?
            .values
            .get(cell.column)?
            .large_value()?
            .clone();
        Some(this.ensure_large_value_window(tab_id, cell.row, cell.column, &large, cx))
    }).flatten();
    if large_value_window.is_none() {
        this.drop_large_value_window(tab_id);
    }
    let cell_detail_drawer = editor.cell_detail_panel.open.then(|| {
        sync_input_value(
            &cell_detail_input,
            &editor.cell_detail_panel.edit_value,
            window,
            cx,
        );
        cell_detail_panel(
            tab_id,
            editor,
            page,
            large_value_window.as_ref(),
            cell_detail_drawer_height,
            cell_detail_input,
            this.temporal_part_input.clone(),
            this.temporal_part_editing,
            colors,
            window,
            cx,
        )
    });
    let drawer_height = cell_detail_drawer_height;
    let footer = (!is_redis_editor).then(|| {
        data_editor_footer(
            tab_id,
            page,
            sql,
            data_sql_footer_selection,
            data_page_input,
            change_count,
            change_sql_preview_open,
            selected_source_row,
            editor.submitting,
            colors,
            window,
            cx,
        )
    });
    let table_area = data_table_area(
        &table_state,
        // 提交与各类加载都会离开 UI 线程，共用同一层遮罩提示，避免用户以为界面失去响应。
        // 已有数据时的重新加载（筛选、翻页、刷新、AI 查询）同样只盖表格区域，
        // 工具栏与筛选面板保持可见可操作。
        filter_applying || editor.submitting || editor.loading,
        if editor.submitting {
            tr("app.data.submitting")
        } else {
            tr("app.data.loading")
        },
        search_open.then(|| {
            data_search_bar(
                tab_id,
                data_search_input,
                search_matches.as_slice(),
                active_search_match,
                search_highlight_all,
                colors,
                cx,
            )
        }),
        change_sql_preview.as_ref().and_then(|preview| {
            change_sql_preview_open
                .then(|| data_change_sql_preview_drawer(tab_id, preview, colors, cx))
        }),
        cell_detail_drawer,
        drawer_height,
        footer,
        colors,
        cx,
    );
    let content = content.child(if editor.table_info.open {
        // DDL 页签用底层编辑器渲染，需要宿主当前主题派生的编辑器配色（见 table_info_ddl_text）。
        let editor_theme = this.editor_theme_for(cx);
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .overflow_hidden()
            .child(table_area)
            .child(table_info_panel(
                tab_id,
                editor,
                page,
                editor_theme,
                window,
                colors,
                cx,
            ))
    } else {
        table_area
    });
    let content = if field_filter_open {
        content.child(field_filter_popover_layer(
            tab_id,
            all_field_names.as_slice(),
            &visible_field_names,
            field_filter_search.as_str(),
            field_filter_search_input,
            colors,
            cx,
        ))
    } else {
        content
    };

    if filter_open {
        if let Some(popover) = filter_popover.filter(|popover| popover.tab_id == tab_id) {
            return content.child(data_filter_popover_layer(
                tab_id,
                page,
                filter_rules.as_slice(),
                sort_rules.as_slice(),
                popover,
                data_filter_value_input,
                data_filter_search_input,
                data_filter_batch_input,
                value_search.as_str(),
                value_search_loading,
                this.data_filter_value_draft.as_ref(),
                colors,
                cx,
            ));
        }
    }

    if let Some(popover) = local_filter_popover.filter(|popover| popover.tab_id == tab_id) {
        return content.child(local_filter_popover_layer(
            page,
            popover,
            local_filter_draft_values,
            local_filter_value_input,
            local_filter_search_input,
            local_filter_search.as_str(),
            colors,
            cx,
        ));
    }

    if local_filter_manager_open {
        return content.child(local_filter_manager_layer(
            tab_id,
            page,
            local_filter_manager_draft_filters,
            local_filter_manager_field_open,
            local_filter_manager_values_open,
            local_filter_manager_field,
            local_filter_search_input,
            local_filter_search.as_str(),
            local_filter_value.as_str(),
            colors,
            cx,
        ));
    }

    content
}

/// 新编辑器承载面板：键鼠/滚动/滚动条/动作分发已全部下沉到 Editor 自身
/// `Editor::render()` 根元素（参考 zed）。宿主这里只保留布局定位 + 配色，
/// 以及 SQL 专属的 `ExecuteQueryShortcut` 绑定（按祖先链从内层 Editor 冒泡到此处）。
fn sql_editor_panel(
    _tab_id: TabId,
    editor: gpui::Entity<editor_component::Editor>,
    colors: UiColors,
    window: &mut Window,
    _cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let _ = window;
    div()
        .id(("sql-editor", editor.entity_id()))
        .flex_1()
        .min_h(px(0.))
        .relative()
        .bg(colors.input_bg)
        // 键上下文仅供 SQL 执行快捷键（ExecuteQueryShortcut）在聚焦编辑器上派发。
        .key_context(editor_component::CONTEXT)
        // SQL 执行快捷键（Run/Select/Explain）分派：读取 `ExecuteQueryShortcut.mode`
        // 原样传给编辑器执行入口，避免被按文本探测的模式替换（整改 6.1/6.2）。
        .on_action({
            let editor = editor.clone();
            move |action: &sql_editor_adapter::ExecuteQueryShortcut,
                  _window: &mut Window,
                  app: &mut gpui::App| {
                editor.update(app, |editor, cx| editor.request_execution(action.mode, cx));
            }
        })
        .child(editor)
}

fn query_editor_content(
    tab_id: TabId,
    editor: &QueryEditorState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let sql_editor = this.query_editor_state(tab_id, editor, window, cx);
    // 新建连接弹框打开时，焦点属于弹框输入框；不要在延迟回调里抢回 SQL 编辑器焦点。
    if this.new_connection_kind.is_none() && !sql_editor.read(cx).is_focused(window) {
        // 这里不能直接在 render 里抢焦点；等本帧挂树完成后再聚焦，才能稳定点亮光标。
        let sql_editor = sql_editor.clone();
        cx.defer_in(window, move |this, window, cx| {
            if this.new_connection_kind.is_none() {
                sql_editor.update(cx, |editor, _cx| editor.focus(window));
            }
        });
    }
    let connection = connection_name(this.controller.state(), editor.connection_id);
    let database = editor
        .database
        .clone()
        .unwrap_or_else(|| tr("app.history.default_database").to_string());
    let accent = this
        .controller
        .state()
        .connections
        .iter()
        .find(|connection| connection.config.id == editor.connection_id)
        .map(|connection| connection_config_color_hex(&connection.config.options))
        .map(connection_color_rgba)
        .unwrap_or_else(|| connection_color_rgba(DEFAULT_CONNECTION_COLOR));
    let output_placement = this.results_placement;
    let workspace = match output_placement {
        ResultsPlacement::Bottom => div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(sql_editor_panel(tab_id, sql_editor.clone(), colors, window, cx))
            .child(query_output_panel(
                tab_id,
                editor,
                output_placement,
                this,
                window,
                colors,
                cx,
            )),
        ResultsPlacement::Right => div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(sql_editor_panel(tab_id, sql_editor.clone(), colors, window, cx)),
            )
            .child(query_output_panel(
                tab_id,
                editor,
                output_placement,
                this,
                window,
                colors,
                cx,
            )),
    };

    div()
        .flex_1()
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, {
            let sql_editor = sql_editor.clone();
            move |_, _, cx| {
                sql_editor.update(cx, |editor, cx| editor.hide_completion(cx));
            }
        })
        .on_mouse_down(MouseButton::Right, {
            let sql_editor = sql_editor.clone();
            move |_, _, cx| {
                sql_editor.update(cx, |editor, cx| editor.hide_completion(cx));
            }
        })
        .child(query_toolbar(
            this,
            tab_id,
            editor,
            sql_editor.clone(),
            connection,
            database,
            accent,
            output_placement,
            colors,
            cx,
        ))
        .child(workspace)
}

/// Redis Workbench 面板：命令输入 + 执行按钮 + 结果列表。
///
/// 不复用 SQL 查询壳的表格结果模型，独立渲染每条 Redis 命令的回复。
fn redis_workbench_content(
    tab_id: TabId,
    workbench: &RedisWorkbenchState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let input = this.redis_workbench_state(tab_id, workbench, window, cx);
    let connection = connection_name(this.controller.state(), workbench.connection_id);
    let accent = this
        .controller
        .state()
        .connections
        .iter()
        .find(|connection| connection.config.id == workbench.connection_id)
        .map(|connection| connection_config_color_hex(&connection.config.options))
        .map(connection_color_rgba)
        .unwrap_or_else(|| connection_color_rgba(DEFAULT_CONNECTION_COLOR));

    // 分栏尺寸按布局取：上下布局用持久化占比 × 可用高度；左右布局用持久化像素宽。
    // 两者来源不同是刻意的 —— 各记各的，切换布局不会把另一种布局的尺寸带偏。
    let placement = this.results_placement;
    let editor_size = match placement {
        ResultsPlacement::Bottom => {
            let split_height = redis_workbench_split_height(window);
            let ratio =
                f32::from(this.controller.state().settings.redis_workbench_editor_ratio) / 100.0;
            workbench_editor_height(ratio, split_height)
        }
        ResultsPlacement::Right => {
            let available = redis_workbench_split_width(this, window);
            let width = f32::from(this.controller.state().settings.redis_workbench_editor_width);
            redis_workbench_editor_width(width, available)
        }
    };
    let input_panel = redis_workbench_input_panel(
        input.clone(),
        editor_size,
        placement,
        colors,
        window,
        cx,
    );
    let split_handle = workbench_split_handle(
        WorkbenchSplitTarget::Redis,
        editor_size,
        placement,
        colors,
        cx,
    );
    let results = redis_workbench_results(tab_id, workbench, this, colors, cx);

    // 编辑器区与结果区的排布方向由布局决定；结果区始终 flex_1 吃掉剩余空间。
    // 注意「右侧」分支刻意不调 `.flex_col()`（外层行方向已给出主轴），
    // 且编辑器侧容器要 `.h_full()` 才能在行方向下撑满高度 —— 与 SQL 结果面板同一处理。
    let split_area = match placement {
        ResultsPlacement::Bottom => div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(input_panel)
            .child(split_handle)
            .child(results),
        ResultsPlacement::Right => div()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .overflow_hidden()
            .child(input_panel)
            .child(split_handle)
            .child(results),
    };

    div()
        .relative()
        .flex_1()
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .child(redis_workbench_toolbar(
            tab_id,
            workbench,
            connection,
            format!("DB {}", workbench.database),
            accent,
            placement,
            colors,
            cx,
        ))
        .child(split_area)
}

/// Redis CLI 终端内容：一个真实 PTY 会话（redis-cli）的终端 surface。
///
/// 与 Redis Workbench 不同，输入/输出都发生在终端 surface 内（不走底部输入框 + 结果卡片）。
/// 顶部仅一条连接/库的收窄信息条，真正交互区由 TerminalComponent 全权负责（渲染网格、光标、
/// 状态栏与危险命令确认条）。会话按 tab 懒建并复用，见 NavicatMain::terminal_component_for。
fn redis_cli_content(
    tab_id: TabId,
    cli: &fluxdb_app::RedisCliState,
    this: &mut NavicatMain,
    _window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let component = this.terminal_component_for(tab_id, cli, cx);
    let connection = connection_name(this.controller.state(), cli.connection_id);
    div()
        .relative()
        .flex_1()
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(28.))
                .border_b_1()
                .border_color(colors.border)
                .bg(colors.panel_alt)
                .flex()
                .items_center()
                .px_3()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(format!("{connection} · DB {} · redis-cli", cli.database)),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                // Entity<TerminalComponent> 自身即完整终端表面（Render 带回 key context / 焦点 / 自绘 canvas）。
                .child(component),
        )
}

/// MongoDB CLI 终端内容：库内执行 shell 语句的进程内 REPL。
///
/// 与 Redis CLI 对称：顶部一条连接/库信息条，交互全部交给 TerminalComponent
/// （grid、光标、状态栏、历史与补全）。
fn mongo_cli_content(
    tab_id: TabId,
    cli: &fluxdb_app::MongoCliState,
    this: &mut NavicatMain,
    _window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let component = this.terminal_component_for_mongo(tab_id, cli, cx);
    let connection = connection_name(this.controller.state(), cli.connection_id);
    div()
        .relative()
        .flex_1()
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(28.))
                .border_b_1()
                .border_color(colors.border)
                .bg(colors.panel_alt)
                .flex()
                .items_center()
                .px_3()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(format!("{connection} · {} · MongoDB Shell", cli.database)),
        )
        .child(div().flex_1().min_h(px(0.)).child(component))
}

/// Redis Pub/Sub 页面：订阅 / 发布实时消息流。
///
/// 会话与消息流由 `navicat_main.pubsub_sessions` 持有（见 pubsub.rs），此处只负责：
/// 顶部工具栏（连接库上下文 + 订阅输入 + 状态）、中央消息流表格（时间 / 通道 / 消息三列
/// + 分页）、底部发布面板。布局借鉴 RedisInsight 的「订阅置顶、消息流表格为主区域、发布
/// 置底」结构，但全部使用 gdb 的 `UiColors` 主题 token（亮 / 暗色均可用）。连接失败 /
/// 连接不存在时呈现明确错误态，不静默失败。
fn redis_pubsub_content(
    tab_id: TabId,
    pubsub: &fluxdb_app::RedisPubSubState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 先确保会话模型与输入框存在；失败（连接不存在）→ 呈现明确错误态。
    let has_session = this.pubsub_session_for(tab_id, pubsub, cx).is_some();
    let connection = connection_name(this.controller.state(), pubsub.connection_id);
    if !has_session {
        return div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_color(colors.muted)
            .text_size(px(13.))
            .child(tr("app.content.pubsub_unavailable"));
    }
    this.ensure_pubsub_inputs(tab_id, window, cx);

    let accent = rgb(0x20c76a);
    let model = this.pubsub_sessions.get(&tab_id).expect("Pub/Sub 会话已建立");
    let status_text = if model.connected {
        tr("app.content.pubsub_connected").to_string()
    } else if model.connecting {
        tr("app.content.pubsub_connecting").to_string()
    } else {
        tr("app.content.pubsub_failed").to_string()
    };
    let status_color = if model.connected {
        accent
    } else if model.connecting {
        colors.muted
    } else {
        rgb(0xff5c5c)
    };
    let error = model.error.clone();
    let subscribed = model.subscribed.clone();
    let messages = model.messages.clone();
    let total = messages.len();
    let page_size = model.message_page_size;
    let page = model.message_page;
    let page_count = total.div_ceil(page_size);
    let subscribe_entity = model.subscribe_input_entity.clone();
    let publish_channel_entity = model.publish_channel_input_entity.clone();
    let publish_message_entity = model.publish_message_input_entity.clone();
    let has_messages = total > 0;

    // ---------- 顶部工具栏：连接上下文 + 状态 + 订阅入口 + 清空 ----------
    // 订阅入口与状态并排置于顶部（RedisInsight 的 header 结构），保持紧凑。
    let header = div()
        .h(px(38.))
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .bg(colors.panel_alt)
        .child(query_context_group(
            connection,
            format!("DB {}", pubsub.database),
            accent,
            colors,
        ))
        .child(
            div()
                .text_size(px(12.))
                .text_color(status_color)
                .child(status_text),
        )
        .child(div().flex_1())
        // 订阅输入 + 订阅按钮。
        .child(
            div()
                .w(px(180.))
                .child(
                    subscribe_entity
                        .map(|entity| Input::new(&entity).into_any_element())
                        .unwrap_or_else(|| div().into_any_element()),
                ),
        )
        .child(
            query_toolbar_icon_button(
                tr("app.content.subscribe"),
                AppIcon::Broadcast,
                true,
                false,
                accent,
                colors,
            )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.pubsub_subscribe_clicked(tab_id, window, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
        // 清空消息。
        .child(
            query_toolbar_icon_button(
                tr("app.history.clear"),
                AppIcon::Trash,
                has_messages,
                false,
                rgb(0xff5c5c),
                colors,
            )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.pubsub_clear_messages(tab_id);
                        cx.notify();
                        cx.stop_propagation();
                    }),
                ),
        );

    // ---------- 已订阅通道条：紧凑横排，点击即取消订阅 ----------
    let mut subscribed_row = div()
        .h(px(30.))
        .flex_none()
        .flex()
        .items_center()
        .gap_1p5()
        .px_3()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .child(
            div()
                .flex_none()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(
                    tr("app.content.subscribed_count")
                        .replace("%{count}", &subscribed.len().to_string()),
                ),
        );
    for (index, ch) in subscribed.iter().enumerate() {
        let ch = ch.clone();
        let channel_for_close = ch.clone();
        subscribed_row = subscribed_row.child(
            div()
                .id(SharedString::from(format!("pubsub-subscribed-{ch}-{index}")))
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .px_1p5()
                .py_0p5()
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .hover(|style| style.bg(colors.hover))
                .tooltip(move |window, cx| {
                    Tooltip::new(tr("app.content.unsubscribe_hint")).build(window, cx)
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.pubsub_unsubscribe(tab_id, &channel_for_close);
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
                .child(
                    div()
                        .max_w(px(120.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(11.))
                        .text_color(colors.text)
                        .child(ch.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child("×"),
                ),
        );
    }

    // ---------- 中央：消息流表格（时间 / 通道 / 消息 三列，条纹行，紧凑行高） ----------
    // 表头：与数据表一致的三列结构，便于对齐分属关系。
    let table_header = div()
        .h(px(28.))
        .flex_none()
        .flex()
        .items_center()
        .px_2()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(pubsub_column_time(
            tr("app.content.column_time").to_string(),
        ))
        .child(pubsub_column_channel(
            tr("app.content.column_channel").to_string(),
        ))
        .child(pubsub_column_message(
            tr("app.content.column_messages").replace("%{count}", &total.to_string()),
        ));

    // 当前页对应的消息（倒序分页：最新消息在第一页首行）。
    let newest_first: Vec<&crate::PubSubMessageView> = messages.iter().rev().collect();
    let page_messages: Vec<&crate::PubSubMessageView> = newest_first
        .iter()
        .skip(page * page_size)
        .take(page_size)
        .copied()
        .collect();

    let mut table_body = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .overflow_x_hidden();
    for (row_index, msg) in page_messages.iter().enumerate() {
        let channel = msg.channel.clone();
        let pattern = msg.pattern.clone();
        let payload = msg.payload.clone();
        let time_label = redis_workbench_time_label(msg.received_at as u64);
        // 条纹行：交替底色增强行辨识，行内按列固定宽度对齐。
        let row_bg = if row_index % 2 == 0 {
            colors.panel_bg
        } else {
            colors.panel_alt
        };
        table_body = table_body.child(
            div()
                .h(px(28.))
                .flex_none()
                .flex()
                .items_center()
                .px_2()
                .border_b_1()
                .border_color(colors.border)
                .bg(row_bg)
                .hover(|style| style.bg(colors.hover))
                .child(pubsub_column_time(time_label).text_color(colors.muted))
                .child(
                    pubsub_column_channel(if let Some(p) = pattern {
                        format!("[{p}] {channel}")
                    } else {
                        channel
                    })
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(accent),
                )
                .child(
                    pubsub_column_message(if payload.is_empty() {
                        tr("app.content.empty_message").to_string()
                    } else {
                        payload
                    })
                    .text_size(px(12.))
                    .text_color(colors.text),
                ),
        );
    }

    // ---------- 分页栏：首页 / 上一页 / 页指示 / 下一页 / 末页 + 每页条数 + 总数 ----------
    let pagination = pubsub_pagination_bar(
        tab_id,
        total,
        page,
        page_count,
        page_size,
        colors,
        cx,
    );

    // ---------- 底部：发布面板（通道 + 内容 + 发布按钮），RedisInsight 置底结构 ----------
    let publish_footer = div()
        .h(px(40.))
        .flex_none()
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .child(
            div()
                .flex_none()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(tr("app.content.publish")),
        )
        .child(
            div()
                .w(px(160.))
                .child(
                    publish_channel_entity
                        .map(|entity| Input::new(&entity).into_any_element())
                        .unwrap_or_else(|| div().into_any_element()),
                ),
        )
        .child(
            div()
                .flex_1()
                .child(
                    publish_message_entity
                        .map(|entity| Input::new(&entity).into_any_element())
                        .unwrap_or_else(|| div().into_any_element()),
                ),
        )
        .child(
            query_toolbar_icon_button(tr("app.content.publish"), AppIcon::Play, true, false, accent, colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.pubsub_publish_clicked(tab_id, window, cx);
                        cx.stop_propagation();
                    }),
                ),
        );

    // 错误态条（连接失败时不静默）。
    let error_bar = error.map(|message| {
        div()
            .h(px(30.))
            .flex_none()
            .flex()
            .items_center()
            .px_3()
            .text_size(px(12.))
            .text_color(rgb(0xff5c5c))
            .bg(rgb(0x2a1415))
            .child(
                tr("app.content.pubsub_error").replace("%{error}", &message),
            )
    });

    div()
        .relative()
        .flex_1()
        .min_w(px(0.))
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .child(header)
        .child(subscribed_row)
        .when_some(error_bar, |this, bar| this.child(bar))
        // 消息流表格为页面主区域，占满剩余空间；其下接分页栏与发布面板。
        .child(table_header)
        .child(table_body)
        .child(pagination)
        .child(publish_footer)
}

/// 表头 / 行内「时间」列容器：固定宽度，右对齐便于数字时间等比阅读。
fn pubsub_column_time(label: String) -> Div {
    div()
        .w(px(150.))
        .flex_none()
        .flex()
        .items_center()
        .pr_2()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_size(px(11.))
        .child(label)
}

/// 表头 / 行内「通道」列容器：固定宽度，文本溢出省略。
fn pubsub_column_channel(label: String) -> Div {
    div()
        .w(px(200.))
        .flex_none()
        .flex()
        .items_center()
        .pr_2()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(label)
}

/// 表头 / 行内「消息」列容器：占据剩余宽度，单行省略，hover 可完整展示（tooltip 由调用侧给）。
fn pubsub_column_message(label: String) -> Div {
    div()
        .w(px(200.))
        .flex_1()
        .min_w(px(0.))
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(label)
}

/// 消息流分页栏：首页 / 上一页 / 页指示 / 下一页 / 末页 + 每页条数 + 总数。
///
/// 全部使用 gdb 主题 token（按钮底、边框、文字），并沿用 `query_toolbar_icon_button` 的
/// 图标按钮风格；每页条数用一组紧凑分段按钮快速切换，避免引入下拉实体状态。
fn pubsub_pagination_bar(
    tab_id: TabId,
    total: usize,
    page: usize,
    page_count: usize,
    page_size: usize,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let can_prev = page > 0 && page_count > 0;
    let can_next = page + 1 < page_count;
    let page_sizes = [25usize, 50, 100, 200];

    // 单页指示文本：第 X / Y 页。
    let page_indicator = if page_count == 0 {
        "0 / 0".to_string()
    } else {
        format!("{} / {}", page + 1, page_count)
    };

    // 复用查询工具栏图标按钮风格构建分页小按钮。
    let pager_button = |label: SharedString,
                        icon: AppIcon,
                        enabled: bool,
                        action: fn(usize, usize) -> usize|
     -> Stateful<Div> {
        let target = action(page, page_count);
        query_toolbar_icon_button(label, icon, enabled, false, rgb(0x1687ff), colors)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if let Some(m) = this.pubsub_sessions.get_mut(&tab_id) {
                        m.message_page = target;
                    }
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
    };

    let first = pager_button(
        tr("app.pagination.first"),
        AppIcon::ChevronsLeft,
        can_prev,
        |_, _| 0,
    );
    // prev: 若已在边界则不动，否则减一。
    let prev = pager_button(
        tr("app.pagination.previous"),
        AppIcon::ChevronLeft,
        can_prev,
        |page, _| page.saturating_sub(1),
    );
    let next = pager_button(
        tr("app.pagination.next"),
        AppIcon::ChevronRight,
        can_next,
        |page, _| page + 1,
    );
    let last = pager_button(
        tr("app.pagination.last"),
        AppIcon::ChevronsRight,
        can_next,
        |_, page_count| page_count.saturating_sub(1),
    );

    div()
        .h(px(34.))
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(tr("app.pagination.page_size")),
        )
        // 每页条数分段选择。
        .children(page_sizes.iter().map(|size| {
            let size_val = *size;
            let active = size_val == page_size;
            let tap_id = tab_id;
            div()
                .id(SharedString::from(format!("pubsub-page-size-{size_val}")))
                .size(px(22.))
                .rounded(colors.radius)
                .border_1()
                .border_color(if active {
                    rgb(0x1687ff)
                } else {
                    colors.border
                })
                .bg(if active { rgb(0x1687ff) } else { colors.panel_bg })
                .cursor_pointer()
                .hover(|style| style.bg(if active { rgb(0x1687ff) } else { colors.hover }))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.))
                .text_color(if active { rgb(0xffffff) } else { colors.text })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if let Some(m) = this.pubsub_sessions.get_mut(&tap_id) {
                            m.message_page_size = size_val;
                            m.message_page = 0;
                        }
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
                .child(size_val.to_string())
        }))
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(tr("app.pagination.total").replace("%{count}", &total.to_string())),
        )
        .child(div().flex_1())
        // 首页 / 上一页。
        .child(first)
        .child(prev)
        .child(
            div()
                .min_w(px(58.))
                .text_size(px(11.))
                .text_color(colors.text)
                .child(page_indicator),
        )
        .child(next)
        .child(last)
}

/// Redis Workbench 顶部工具栏：连接 / 数据库上下文 + 运行按钮（带 loading）。
fn redis_workbench_toolbar(
    tab_id: TabId,
    workbench: &RedisWorkbenchState,
    connection: String,
    database: String,
    accent: gpui::Rgba,
    placement: ResultsPlacement,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let has_text = !workbench.text.trim().is_empty();
    let running = workbench.running;
    let has_results = !workbench.executions.is_empty();
    // 运行前可能触发危险命令二次确认，需要把待执行文本提前取出供闭包判断。
    let pending_text = workbench.text.clone();
    div()
        .h(px(40.))
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .bg(colors.panel_alt)
        .child(query_context_group(connection, database, accent, colors))
        .child(
            query_toolbar_icon_button(
                if running {
                    tr("app.content.running")
                } else {
                    tr("app.content.run")
                },
                AppIcon::Play,
                has_text && !running,
                running,
                rgb(0x20c76a),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if has_text && !running {
                        // 含破坏性命令时先弹确认框；确认后才派发执行。
                        if this.request_redis_dangerous_confirmation(tab_id, &pending_text, cx) {
                            cx.stop_propagation();
                        } else {
                            this.run_redis_workbench(tab_id, None, cx);
                            cx.stop_propagation();
                        }
                    }
                }),
            ),
        )
        .child(
            query_toolbar_icon_button(
                tr("app.content.clear_results"),
                AppIcon::Trash,
                has_results && !running,
                false,
                rgb(0xff5c5c),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.dispatch(AppCommand::ClearRedisWorkbenchResults(tab_id), cx);
                    cx.stop_propagation();
                }),
            ),
        )
        // 撑开左侧操作组，让结果区布局切换按钮单独贴最右（与 SQL 编辑器工具栏同一位置）。
        // 这里原先还有一句「Redis 命令执行器」状态文字，已去掉：运行中状态由「运行」按钮
        // 自身的 loading 态表达（见上），文字是重复信息。
        .child(div().flex_1())
        .child(
            // 结果区布局切换：与 SQL 编辑器工具栏右上角同一个按钮、同一套图标。
            query_toolbar_icon_button(
                query_output_layout_toggle_label(placement),
                query_output_layout_toggle_icon(placement),
                true,
                false,
                rgb(0x45a3ff),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    // 只改运行时值，不落盘：设置里的「查询结果默认布局」才是启动默认值。
                    this.results_placement = this.results_placement.toggled();
                    // 布局一变，旧拖拽起点记录的尺寸属于另一种轴（高/宽），必须丢弃，
                    // 否则下一次拖动会把残留的高度当成宽度用。
                    this.workbench_panel_resize_start = None;
                    cx.notify();
                    cx.stop_propagation();
                }),
            ),
        )
}


/// 命令输入区：多行代码输入框。`下方`布局下由高度决定，`右侧`布局下由宽度决定，
/// 两种布局都可用对应的分栏手柄拖动调整大小。
///
/// 布局要点：外层容器必须在**流轴**上给出确定尺寸 —— `Editor::render` 的根节点是
/// `.relative().size_full()`，其 `100%` 需要父级先有确定尺寸才能解算出像素值。
/// 因此「下方」给 `.h(px(size))`，「右侧」给 `.w(px(size))` 并额外 `.h_full()`：
/// 右侧布局下外层是行方向，不写 `.h_full()` 的话高度只有内容高，编辑器撑不满整列。
fn redis_workbench_input_panel(
    editor: Entity<editor_component::Editor>,
    size: f32,
    placement: ResultsPlacement,
    colors: UiColors,
    _window: &mut Window,
    _cx: &mut Context<NavicatMain>,
) -> Div {
    // 键鼠/滚动/滚动条/动作分发已下沉到 Editor::render()；此处仅保留布局定位 + 配色。
    // 分隔线画在朝向结果区的那一侧：上下分栏画底边，左右分栏画右边。
    // 返回具体 `Div` 而非 `impl IntoElement`：后者会捕获 `_cx` 的生命周期，
    // 调用方一旦把结果存进局部变量就会与后续的 `cx` 借用冲突。
    let panel = div()
        .border_color(colors.border)
        .bg(colors.input_bg);
    let panel = match placement {
        // 上下分栏允许收缩（但保留像素下限）：按视口估算的分栏高可能大于实际
        // 可用高，固定不缩会把结果区压到下限以下。右侧布局由宽度约束，保持不缩。
        ResultsPlacement::Bottom => panel
            .h(px(size))
            .min_h(px(WORKBENCH_EDITOR_MIN_HEIGHT))
            .flex_initial()
            .border_b_1(),
        ResultsPlacement::Right => panel
            .w(px(size))
            .h_full()
            .min_w(px(0.))
            .flex_none()
            .border_r_1(),
    };
    panel.child(editor)
}

/// Workbench 分栏拖动手柄：居中一排 3 个圆点（对齐 RedisInsight 的 grip 观感）。
/// 拖动改变分栏大小并即时写回持久化设置，重开 Workbench 仍可恢复。
/// Redis「下方」布局拖高度（`redis_workbench_editor_ratio`），「右侧」拖宽度
/// （`redis_workbench_editor_width`）；MongoDB Workbench 只支持上下分栏。
fn workbench_split_handle(
    target: WorkbenchSplitTarget,
    editor_size: f32,
    placement: ResultsPlacement,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    // 手柄粗细在两个轴向上相同，只是方向不同。
    const SPLIT_HANDLE_THICKNESS: f32 = 7.;
    let dot = colors.muted;
    // grip 三点在两个轴向上都要居中排布：上下分栏的条是横的（点横排），
    // 左右分栏的条是竖的（点必须竖排，否则 7px 宽里三个点会横向溢出）。
    let grip = div()
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .child(div().size(px(2.)).rounded_full().bg(dot))
        .child(div().size(px(2.)).rounded_full().bg(dot))
        .child(div().size(px(2.)).rounded_full().bg(dot));
    let handle = match placement {
        ResultsPlacement::Bottom => div().h(px(SPLIT_HANDLE_THICKNESS)).cursor_ns_resize().child(grip),
        ResultsPlacement::Right => div()
            .w(px(SPLIT_HANDLE_THICKNESS))
            .h_full()
            .cursor_col_resize()
            .child(grip.flex_col()),
    };

    handle
        .id(match target {
            WorkbenchSplitTarget::Redis => "redis-workbench-split-handle",
            WorkbenchSplitTarget::Mongo => "mongo-workbench-split-handle",
        })
        .flex_none()
        .bg(colors.panel_bg)
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                // 起点记下目标、布局与两个轴的坐标：布局可能在工作台间切换、
                // 松手也可能不触发（见 `WorkbenchPanelResizeStart` 的说明）。
                this.workbench_panel_resize_start = Some(WorkbenchPanelResizeStart {
                    target,
                    placement,
                    x: f32::from(event.position.x),
                    y: f32::from(event.position.y),
                    size: editor_size,
                });
                cx.stop_propagation();
            }),
        )
        .on_drag(WorkbenchPanelResizeDrag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_drag_move(cx.listener(
            move |this, event: &DragMoveEvent<WorkbenchPanelResizeDrag>, window, cx| {
                let Some(start) = this.workbench_panel_resize_start else {
                    return;
                };
                // 一律按**起点记录的目标/布局**换算，不读实时值：目标与轴必须配对。
                let mut settings = this.controller.state().settings.clone();
                match (start.target, start.placement) {
                    (WorkbenchSplitTarget::Redis, ResultsPlacement::Bottom) => {
                        // 增量 = 当前 Y − 按下 Y：手柄向下拖（Y 增大）→ 编辑器区变高。
                        let delta = f32::from(event.event.position.y) - start.y;
                        let split = redis_workbench_split_height(window);
                        let new_size = workbench_clamp_editor_size(start.size + delta, split);
                        // 换算回整数占比写入内存设置，触发重渲染即时反馈；落盘推迟到松开鼠标。
                        settings.redis_workbench_editor_ratio =
                            (new_size / split * 100.0).round().clamp(0.0, 100.0) as u8;
                    }
                    (WorkbenchSplitTarget::Redis, ResultsPlacement::Right) => {
                        // 增量 = 当前 X − 按下 X：编辑器区在**左**，手柄向右拖 → 编辑器区变宽。
                        // 注意这与 SQL 结果面板相反（那块面板在右侧、向左生长，
                        // 所以那边是 `start.x - current.x`），别照抄。
                        let delta = f32::from(event.event.position.x) - start.x;
                        let available = redis_workbench_split_width(this, window);
                        let new_size =
                            redis_workbench_editor_width(start.size + delta, available);
                        settings.redis_workbench_editor_width =
                            new_size.round().clamp(0.0, f32::from(u16::MAX)) as u16;
                    }
                    (WorkbenchSplitTarget::Mongo, _) => {
                        // Mongo Workbench 只有上下分栏，恒定按 Y 轴换算。
                        let delta = f32::from(event.event.position.y) - start.y;
                        let split = mongo_workbench_split_height(window);
                        let new_size = workbench_clamp_editor_size(start.size + delta, split);
                        settings.mongo_workbench_editor_ratio =
                            (new_size / split * 100.0).round().clamp(0.0, 100.0) as u8;
                    }
                }
                let _ = this.controller.dispatch(AppCommand::SaveSettings(settings));
                cx.notify();
                cx.stop_propagation();
            },
        ))
        // 落盘只在松手时做。`on_mouse_up` 仅在指针仍悬停手柄上时触发，
        // 而正常拖拽手势是在结果区松手，所以必须补一个 `on_mouse_up_out`
        // （非悬停时触发）才真的落盘 —— 否则分栏大小拖完一重启就丢。
        // 仅消费已开始的拖拽；外部松手回调在捕获阶段运行，无拖拽时不得拦截其他控件的点击。
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                if this.workbench_panel_resize_start.take().is_some() {
                    let _ = this.storage.save_settings(&this.controller.state().settings);
                    cx.stop_propagation();
                }
            }),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                if this.workbench_panel_resize_start.take().is_some() {
                    let _ = this.storage.save_settings(&this.controller.state().settings);
                    cx.stop_propagation();
                }
            }),
        )
}

/// 结果区：空态 / 执行中 / 错误态，或按「一执行一条记录」展示的执行记录列表。
/// `this` 用于读取 NavicatMain 自身的 hover 记录态；不能在 render 期间通过
/// `cx.entity().read(cx)` 自读（render 时 NavicatMain 已被租用，会触发 double-lease panic）。
fn redis_workbench_results(
    tab_id: TabId,
    workbench: &RedisWorkbenchState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let body: gpui::AnyElement = if workbench.executions.is_empty() {
        if workbench.running {
            redis_workbench_loading_state(colors).into_any_element()
        } else if let Some(error) = workbench.error.as_ref() {
            // 重试与工具栏「运行」同路：含破坏性命令时先弹二次确认。
            let pending_text = workbench.text.clone();
            let retry = Button::new(("retry-redis-workbench", tab_id.0))
                .label(tr("app.common.retry"))
                .small()
                .outline()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.request_redis_dangerous_confirmation(tab_id, &pending_text, cx) {
                        this.run_redis_workbench(tab_id, None, cx);
                    }
                }))
                .into_any_element();
            query_output_error_state(tab_id, error, Some(retry), cx, colors).into_any_element()
        } else {
            redis_workbench_empty_state(colors).into_any_element()
        }
    } else {
        // 已有执行记录时始终展示记录列表；新执行完成后把新记录追加到列表末尾。
        redis_workbench_records(tab_id, workbench, this, colors, cx).into_any_element()
    };

    // 横向：右侧布局下结果区会变窄，`.min_w(px(0.))` 才能让它真正收缩
    // （否则 min-width:auto 取内容最小宽，超宽内容会把结果区顶出去）。
    // 注意 `.overflow_y_scrollbar()` 返回的 `Scrollable` 会按轴强制 overflow，
    // 再把样式转发到外层 wrapper —— 横向裁剪必须由这层朴素容器承担。
    let scroller = div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .child(body);
    // 外层必须是 flex 容器，内层的 `flex_1` 才会生效；否则滚动区高度退化成内容高，
    // 超出部分会被这里的 `overflow_hidden` 直接裁掉且无法滚动。
    div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(WORKBENCH_RESULT_MIN_HEIGHT))
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(scroller)
        .into_any_element()
}

/// 按执行顺序逐条渲染执行记录。每条记录：折叠 / 展开 + 状态 + 命令文本 + 时间 + 耗时 + 右侧 Run / Delete。
fn redis_workbench_records(
    tab_id: TabId,
    workbench: &RedisWorkbenchState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let running = workbench.running;
    // 结果列表铺在更深的结果区底上，卡片之间留出纵向间距，形成清晰的“黑底嵌卡片”层次。
    let mut list = div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .gap_2()
        .p_2()
        .bg(colors.status_bg);
    for execution in &workbench.executions {
        let collapsed = workbench.collapsed.contains(&execution.id);
        list = list.child(redis_workbench_record_row(
            tab_id,
            execution,
            running,
            collapsed,
            &workbench.json_views,
            this,
            colors,
            cx,
        ));
    }
    list
}

/// Redis Workbench 记录头部轻量纯图标按钮：无外边框、小尺寸（18px 命中区 / 14px 图标），
/// hover 仅轻微背景高亮（透明 → 主题 hover 底），明暗主题经 `colors.hover` 适配。
/// 返回 `Stateful<Div>`，调用方可继续链式绑定 `on_mouse_down`（各自 stop_propagation）。
fn redis_workbench_icon_button(
    execution_id: u64,
    tooltip_label: impl Into<SharedString>,
    icon: AppIcon,
    icon_color: gpui::Rgba,
    colors: UiColors,
    enabled: bool,
) -> Stateful<Div> {
    let tooltip_label = tooltip_label.into();
    // 禁用（运行中）时图标降为 muted，且不显示手型 / tooltip。
    let icon_color = if enabled { icon_color } else { colors.muted };
    div()
        .id(SharedString::from(format!(
            "redis-workbench-icon:{execution_id}:{tooltip_label}"
        )))
        .size(px(18.))
        .flex_none()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .justify_center()
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .tooltip(move |window, cx| {
                    Tooltip::new(tooltip_label.clone()).build(window, cx)
                })
        })
        .child(app_icon(icon, 14., icon_color))
}

/// 单条执行记录卡片：一次提交中的一条命令 = 一张卡（执行单元 = 单条命令）。
/// 卡片头部整栏可点击切换折叠/展开；左侧箭头仅是折叠状态指示图标（非按钮）。
/// 右侧提供独立的 Run（重跑）与 Delete（只删当前）图标，点击不触发整栏折叠。
/// 身体展示该命令的 reply，结果区点击不触发折叠。折叠后只显示头部；运行期间禁用交互。
fn redis_workbench_record_row(
    tab_id: TabId,
    execution: &CommandWorkbenchExecution,
    running: bool,
    collapsed: bool,
    json_views: &BTreeSet<(u64, usize)>,
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let execution_id = execution.id;
    let workbench_json_views = json_views;
    let command_text = execution.text.clone();
    // 记录层 JSON 判定与视图态：每条执行记录恰好承载一条命令（command_index 恒为 0），
    // 因而把 `Text / JSON` 切换放在"记录头部运行图标之前"，状态沿用 `json_views`，不引入重复源。
    let is_json_cmd = execution
        .commands
        .first()
        .map(redis_workbench_is_json_command)
        .unwrap_or(false);
    let is_json_view = json_views.contains(&(execution_id, 0));
    // 供 Popover 内容闭包内 dispatch 命令使用（NavicatMain 弱引用），复用既有状态源。
    let navicat_view = cx.entity().downgrade();
    let success = execution.summary.failed == 0;
    let status_color = if success {
        rgb(0x20c76a)
    } else {
        rgb(0xff5c5c)
    };
    let status_label = if success {
        tr("app.history.success")
    } else {
        tr("app.history.failed")
    };
    // 耗时（毫秒级，RedisInsight 风格 `N msec`）与执行时刻（完整日期时间，来自 started_at_unix_secs）双展示。
    // 后端仅存整数毫秒（`elapsed_ms: u64`），如实展示整数 `msec`，不拼接假的小数精度。
    let elapsed_label = format!("{} msec", execution.elapsed_ms);
    let time_label = redis_workbench_time_label(execution.started_at_unix_secs);
    // 折叠箭头：展开用 ChevronDown，折叠用 ChevronRight。
    let collapse_icon = if collapsed {
        AppIcon::ChevronRight
    } else {
        AppIcon::ChevronDown
    };
    // 记录下的每条子命令独立展示 reply，复用原有 command row 渲染（折叠时整段不渲染）。
    // 身体直接铺在卡片基色的深色底上（见下方外层卡片），与头部横条形成明显分层。
    // 结果区设置最大高度：超长 reply / 长列表在卡片内部纵向滚动查看，只滚动结果正文本身，
    // 不影响外层记录列表与整页布局。
    // 注意：这里用原生 `overflow_y_scroll()` 而非 `overflow_y_scrollbar()`（Scrollable 封装）。
    // Scrollable 的 render 会给结果区套 `size_full()`（height:100%）并让内部滚动区 `flex_1()`，
    // 在 auto 高度的卡片 flex 列里会压垮自身内容高度 → 结果内容较短时整卡被压矮。
    // 改为原生 overflow_y_scroll 后，body 保持内容感知（flex_none：0 0 auto，不做高度收缩），
    // 内容短时即自然高度、卡片不变矮；内容超过 max_h 阈值时封顶在阈值内纵向滚动。
    // 也无需 `min_h(px(0.))`：body 不参与外部收缩，不会因 max-height 约束而失效。
    // `overflow_y_scroll()` 是 gpui `StatefulInteractiveElement` 的 trait 方法，需在带 `.id()` 的
    // 有状态元素上调用（id 需记录级唯一）——这里也要保留唯一 id 以便 `.id()` 调用合法。
    let mut body = div()
        .id(("redis-wb-record-body", execution_id))
        .flex_none()
        .max_h(px(REDIS_WB_RECORD_BODY_MAX_HEIGHT))
        .border_t_1()
        .border_color(colors.border_soft)
        .flex()
        .flex_col()
        .overflow_y_scrollbar();
    for (command_index, item) in execution.commands.iter().enumerate() {
        let is_json_view = workbench_json_views.contains(&(execution_id, command_index));
        body = body.child(redis_workbench_command_row(item, is_json_view, colors));
    }

    // RedisInsight 式卡片：外层圆角卡片（深黑底）+ 头部深灰横条 + 下方更深的黑底结果区；
    // `overflow_hidden` 裁切圆角，头部与结果区分层明显。
    // 头部为三段式均衡布局，从左到右依次：左侧命令（整栏点击折叠，箭头仅作折叠状态指示）、
    // 中间 meta 状态/时间/耗时、右侧紧凑操作（Text/JSON 下拉 + Run + Delete）。
    // 左侧与右侧动作区均为 `flex_1`，使中间 meta 时间区在可用宽度内居中、三段分布更均衡；
    // 动作区 `justify_end` 保证按钮始终贴右，按钮间用 `gap_2` 保持合适间距。
    // 右侧按钮各自 stop_propagation，点击不触发整栏折叠。
    // 直接读 NavicatMain 自身字段判断 hover 态：不能经 `cx.entity().read(cx)` 自读，
    // render 期间 NavicatMain 已被租用（更新中），自读会触发
    // 「cannot read NavicatMain while it is already being updated」double-lease panic。
    let row_hovered = this.redis_workbench_hovered_record == Some((tab_id, execution_id));
    // 复制命令用独立的克隆，供 hover 浮现的复制按钮闭包使用，避免 move 走下方动作区要用的 `command_text`。
    let copy_command_text = command_text.clone();
    // 头部带唯一 id，使元素为有状态（`Stateful<Div>`），才可绑定 `on_hover`；
    // 也用作「复制命令」hover 浮现的记录级唯一标识。
    let header = div()
        .id(("redis-wb-record-header", execution_id))
        .h(px(34.))
        .px_2()
        .gap_2()
        .bg(colors.panel_alt)
        .cursor_pointer()
        .flex()
        .items_center()
        // hover 态记录：仅用于让「复制命令」按钮在本行 hover 时浮现（不常驻抢标题行空间），
        // 不改变按钮 / 折叠语义。hover 离开即清除。
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            this.redis_workbench_hovered_record = if *hovered {
                Some((tab_id, execution_id))
            } else {
                None
            };
            cx.notify();
        }))
        // 头部整栏点击切换折叠/展开；运行期间禁用。右侧按钮各自 stop_propagation，不会冒泡到此处。
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if running {
                    return;
                }
                this.dispatch(
                    AppCommand::ToggleRedisWorkbenchRecordCollapse {
                        tab_id,
                        execution_id,
                    },
                    cx,
                );
                cx.stop_propagation();
            }),
        )
        // 左侧：折叠状态指示箭头 + 命令文本（可收缩、过长截断省略），整体 flex_1 占据剩余宽度。
        // 「复制命令」仅在整行 hover 时浮现，避免常驻占位。
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .items_center()
                .gap_1()
                .child(app_icon_box(collapse_icon, 24., 12., colors.muted).flex_none())
                .child(
                    div()
                        .min_w(px(0.))
                        .flex_shrink(1.)
                        .text_size(px(12.))
                        .font_family(crate::PLATFORM_MONOSPACE_FONT)
                        .text_color(colors.text)
                        .overflow_x_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(single_line_summary_text(command_text.clone())),
                )
                .when(row_hovered && !running, |this| {
                    // 复制命令：hover 浮现的轻量纯图标，点击复制本记录完整命令文本；stop_propagation 不触发折叠。
                    this.child(
                        redis_workbench_icon_button(
                            execution_id,
                            tr("app.content.copy_command"),
                            AppIcon::Copy,
                            colors.muted,
                            colors,
                            true,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener({
                                let copy_command_text = copy_command_text.clone();
                                move |this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copy_command_text.clone(),
                                    ));
                                    this.show_message(
                                        tr("app.content.copied_command"),
                                        AppMessageKind::Success,
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }
                            }),
                        ),
                    )
                }),
        )
        // 中间 meta 区：运行状态 + 执行时间 + 耗时，状态紧挨时间/耗时之前、紧凑成一组，靠右不抢动作区。
        .child(redis_workbench_record_meta(
            status_label,
            status_color,
            &time_label,
            &elapsed_label,
            colors,
        ))
        // 右侧动作区：Text/JSON 下拉（仅 JSON 命令）+ Run（重跑）+ Delete（删除），紧凑排列。
        .child(redis_workbench_record_actions(
            tab_id,
            execution_id,
            command_text,
            running,
            is_json_cmd,
            is_json_view,
            navicat_view,
            colors,
            cx,
        ));

    div()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        // 卡片基色用更深的结果区底色，头部横条（panel_alt）叠在上方形成“黑底嵌卡”层次。
        .bg(colors.input_bg)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(header)
        .when(!collapsed, |this| this.child(body))
}

/// 记录头部中间 meta 区：运行状态 + 执行时间 + 耗时，紧凑成一组、状态紧挨时间/耗时之前。
/// 状态用成功 / 失败色突出，时间与耗时用弱化 muted；整组 `flex_none` 靠右、不抢动作区。
/// 颜色全走 `UiColors` 与主题绿 / 红固定语义色，明暗主题均适配。
fn redis_workbench_record_meta(
    status_label: impl Into<SharedString>,
    status_color: gpui::Rgba,
    time_label: &str,
    elapsed_label: &str,
    colors: UiColors,
) -> Div {
    let status_label = status_label.into();
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap_1p5()
        .child(
            div()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(status_color)
                .child(status_label),
        )
        // 完整执行时间 + 毫秒耗时：如 `2026-01-01 14:03:22 · 16 msec`，整段为 ASCII（规避中英文混排 bug）。
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .whitespace_nowrap()
                .child(format!("{time_label} · {elapsed_label}")),
        )
}

/// 记录头部右侧动作区：Text/JSON 下拉（仅 JSON 命令）+ Run（重跑）+ Delete（删除）。
/// 按钮各自 `stop_propagation` 避免触发整栏折叠；运行期间 Run / Delete 禁用。
/// `flex_1 + justify_end` 让动作区从右侧边缘占满整行，配合左侧命令区的 `flex_1`，
/// 使中间 meta 时间区在三段间居中、分布更均衡（贴合 RedisInsight 头部）；
/// 按钮间用 `gap_2` 适度拉开水平间距，避免挤在一起。
/// 复用 `redis_workbench_icon_button` 轻量纯图标方案，明暗主题经 `colors` 适配。
fn redis_workbench_record_actions(
    tab_id: TabId,
    execution_id: u64,
    command_text: String,
    running: bool,
    is_json_cmd: bool,
    is_json_view: bool,
    navicat_view: WeakEntity<NavicatMain>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .children({
            let mut icons: Vec<gpui::AnyElement> = Vec::new();
            // 记录层的 `Text / JSON` 切换下拉：置于运行图标之前（仅 JSON 命令显示）。
            // 切换入口从命令行标题行迁移到"结果头部运行图标前"，与 RedisInsight 结果头部
            // 一致；状态沿用 `json_views`（每条执行记录恰好一条命令，command_index 恒为 0）。
            // 外层包一层 `stop_propagation`：记录头整栏绑定了折叠 on_mouse_down，点击下拉
            // 需终止冒泡，避免同时触发整卡折叠。
            if is_json_cmd {
                icons.push(
                    div()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .child(redis_workbench_json_view_dropdown(
                            tab_id,
                            execution_id,
                            0,
                            is_json_view,
                            navicat_view.clone(),
                            colors,
                        ))
                        .into_any_element(),
                );
            }
            // 单条 Run：重跑当前记录（危险命令二次确认约束，运行期间禁用），轻量纯图标。
            icons.push(
                redis_workbench_icon_button(
                    execution_id,
                    tr("app.history.rerun"),
                    AppIcon::Play,
                    colors.muted,
                    colors,
                    !running,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let command_text = command_text.clone();
                        move |this, _, _, cx| {
                            if running {
                                return;
                            }
                            if this.request_redis_record_rerun_confirmation(
                                tab_id,
                                &command_text,
                                execution_id,
                                cx,
                            ) {
                                cx.stop_propagation();
                            } else {
                                this.run_redis_workbench(tab_id, Some(execution_id), cx);
                                cx.stop_propagation();
                            }
                        }
                    }),
                )
                .into_any_element(),
            );
            // 单条 Delete：只删除当前这条记录（运行期间禁用，避免中断执行），轻量纯图标。
            icons.push(
                redis_workbench_icon_button(
                    execution_id,
                    tr("app.common.delete"),
                    AppIcon::Trash,
                    rgb(0xff5c5c),
                    colors,
                    !running,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if running {
                            return;
                        }
                        this.dispatch(
                            AppCommand::DeleteRedisWorkbenchRecord {
                                tab_id,
                                execution_id,
                            },
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                )
                .into_any_element(),
            );
            icons
        })
}

/// 把 Unix 秒时间戳格式化为本地完整日期时间 `YYYY-MM-DD HH:MM:SS`，
/// 用于每条执行记录头部的时间展示（复用后端 `started_at_unix_secs` 的开始时间）。
/// 合成 / 测试数据可能传 0，此时转空字符串，避免展示纪元时间。
fn redis_workbench_time_label(unix_secs: u64) -> String {
    if unix_secs == 0 {
        return String::new();
    }
    chrono::DateTime::from_timestamp(unix_secs as i64, 0)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// 空态：尚未执行任何命令。
fn redis_workbench_empty_state(colors: UiColors) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .text_color(colors.muted)
        // 空态提示：整句保持纯中文，避免与英文命令示例混排。
        // gpui 0.2.2 对「中文 + ASCII 命令」同一文本节点内的跨字体 run 切分存在
        // 字节越界 bug（shape_line 处 panics），故不在此行混入 GET/SET 等英文示例。
        .child(tr("app.content.command_hint"))
}

/// 执行中占位态。
fn redis_workbench_loading_state(colors: UiColors) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .child(loading_spinner_with_color(18., colors.muted))
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors.muted)
                .child(tr("app.content.executing")),
        )
}

/// 单条命令的结果行。
fn redis_workbench_command_row(
    item: &CommandExecutionItem,
    is_json_view: bool,
    colors: UiColors,
) -> Div {
    let status_color = match item.status {
        CommandExecutionStatus::Success => rgb(0x20c76a),
        CommandExecutionStatus::Failed => rgb(0xff5c5c),
        CommandExecutionStatus::Skipped => colors.muted,
    };
    let status_label = match item.status {
        CommandExecutionStatus::Success => SharedString::new_static("OK"),
        CommandExecutionStatus::Failed => tr("app.content.command_failed"),
        CommandExecutionStatus::Skipped => tr("app.content.command_skipped"),
    };
    // 模式切换入口已迁移到"记录头部运行图标之前"的紧凑下拉（见 `redis_workbench_json_view_dropdown`，
    // 由外层 `redis_workbench_record_row` 持有），命令行标题行不再重复展示 `Text / JSON` 切换控件。
    // 开启 JSON 视图时用 `JsonComponent::Preview` 只读渲染结构化高亮（非法 JSON 自动回退原文，绝不留空）；
    // 默认 Text 态沿用纯文本展示。
    let is_json_cmd = redis_workbench_is_json_command(item);
    let reply_body = if is_json_cmd {
        let mut component = JsonComponent::new(JsonEditorConfig::default());
        component.load(&command_reply_display(&item.reply));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(if is_json_view {
                component.render_preview(colors).into_any_element()
            } else {
                redis_workbench_reply_row(&item.reply, colors).into_any_element()
            })
    } else {
        redis_workbench_reply_row(&item.reply, colors)
    };
    div()
        .border_b_1()
        .border_color(colors.border_soft)
        .px_3()
        .py_2()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(status_color)
                        .child(status_label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(13.))
                        .font_family(crate::PLATFORM_MONOSPACE_FONT)
                        .text_color(colors.text)
                        .overflow_x_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(item.command.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(format!("{}ms", item.elapsed_ms)),
                )
        )
        .child(reply_body)
}

/// 判断该子命令是否属于 JSON 命令（首个 argv 分词大写后以 `JSON.` 开头）。
///
/// 非 JSON 命令的结果不展示 `Text / JSON` 切换；This 只对 Redis 的 `JSON.*` 系列命令启用。
fn redis_workbench_is_json_command(item: &CommandExecutionItem) -> bool {
    item.argv_preview
        .first()
        .map(|s| s.to_ascii_uppercase().starts_with("JSON."))
        .unwrap_or(false)
}

/// 记录头部运行图标之前的 `Text / JSON` 展示模式下拉（紧凑 RedisInsight 风格）。
///
/// 切换入口从原来的结果内容区分段按钮迁移到结果层头部右侧、运行图标之前：触发按钮常驻显示
/// 当前模式（`Text` / `JSON`）并带下拉箭头，点击弹出只有两项的小菜单，选中项带 check 标记。
/// 状态复用原有 `json_views` 集合（按 `execution_id + command_index` 记忆，记录级此处
/// command_index 恒为 0），点击任一菜单项 dispatch `ToggleRedisWorkbenchJsonView`，由既有状态源
/// 驱动内容区立即刷新，不引入重复状态源。
/// 复用 `gpui_component` 的 `Popover`，触发按钮与菜单样式都用主题 `UiColors`，兼容明暗主题、不写死颜色。
/// 仅 JSON 命令渲染（由调用方 `is_json_cmd` 控制）。
fn redis_workbench_json_view_dropdown(
    tab_id: TabId,
    execution_id: u64,
    command_index: usize,
    is_json_view: bool,
    view: WeakEntity<NavicatMain>,
    colors: UiColors,
) -> Popover {
    // 触发按钮当前模式文案；未选中（Text）用 muted、选中（JSON）用主文本色做视觉区分。
    let current_label = if is_json_view { "JSON" } else { "Text" };
    // 触发按钮复用 gpui_component 的 `Button`（满足 Popover 的 `Selectable` 约束），
    // ghost + xsmall 紧凑尺寸；按钮本身不绑定点击，Popover 外层自动处理"点击触发开合"。
    // 用自定义 muted 箭头代替 Button 内建 `dropdown_caret`（后者 hover 会继承主题强调色），保持中性低干扰。
    let trigger = Button::new(SharedString::from(format!(
        "redis-wb-json-view-trigger-{execution_id}-{command_index}"
    )))
    .ghost()
    .xsmall()
    .h(px(20.))
    .px_1p5()
    .gap_1()
    .child(
        div()
            .text_size(px(11.))
            .font_family(crate::PLATFORM_MONOSPACE_FONT)
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(if is_json_view { colors.text } else { colors.muted })
            .child(current_label),
    )
    .child(app_icon(AppIcon::ChevronDown, 10., colors.muted));

    // 菜单项配置：`(文案, 是否选中, 目标模式)`。
    let items: [(&'static str, bool, bool); 2] = [
        ("Text", !is_json_view, false),
        ("JSON", is_json_view, true),
    ];
    // 构建单个菜单项：左侧 check 标记占位 + 文案。点击时若目标模式与当前不同则 dispatch 切换，
    // 无论是否切换都立即关闭下拉（贴近原生下拉交互）。
    // 先 `view.clone()` / `popover.clone()` 各留一份给内层闭包：让 make_item 捕获不被子闭包 move 走，
    // 从而保持 `Fn` 可被 `.map` 多次调用（内容闭包每次渲染都会重建菜单项）。
    let make_item = move |(label, active, target_json): (&'static str, bool, bool),
                          popover: WeakEntity<PopoverState>| {
        let fg = if active { colors.text } else { colors.muted };
        let view = view.clone();
        let row = div()
            .px_1p5()
            .py_1()
            .rounded(colors.radius * 0.5)
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .hover(move |style| style.bg(colors.hover))
            // 选中态 check 标记占位；未选中留空，保持两行视觉对齐。
            .child(
                div()
                    .w(px(14.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(if active {
                        app_icon(AppIcon::Check, 12., colors.text).into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            )
            .child(
                div()
                    .w_full()
                    .text_size(px(12.))
                    .font_family(crate::PLATFORM_MONOSPACE_FONT)
                    .font_weight(if active {
                        gpui::FontWeight::SEMIBOLD
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .text_color(fg)
                    .child(label),
            );
        row.on_mouse_down(
            MouseButton::Left,
            move |_, window, cx| {
                // 与旧分段按钮语义一致：仅目标模式与当前不同才触发切换，保持幂等。
                if target_json != is_json_view {
                    let _ = view.update(cx, |this, cx| {
                        this.dispatch(
                            AppCommand::ToggleRedisWorkbenchJsonView {
                                tab_id,
                                execution_id,
                                command_index,
                            },
                            cx,
                        );
                    });
                }
                // 点击任意菜单项后立即关闭下拉：通过 Popover 状态实体调用 `dismiss`，
                // 让视图切换结果即时可见，交互更贴近原生下拉菜单。
                let _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
            },
        )
    };

    Popover::new(gpui::ElementId::Name(SharedString::from(format!(
        "redis-wb-json-view-{execution_id}-{command_index}"
    ))))
    // 关闭内置外观：自行用紧凑容器控制面板尺寸，避免默认大卡片式留白。
    .appearance(false)
    .anchor(Anchor::TopRight)
    .trigger(trigger)
    // 内容：小尺寸两行菜单，选中项带 check、加粗、主文本色，hover 高亮衬托可点性。
    // `cx.entity()` 拿到 Popover 自身的 `PopoverState` 弱引用，交给菜单项点击后调用 `dismiss` 关闭下拉。
    .content(move |_, _, cx| {
        let popover = cx.entity().downgrade();
        div()
            .w(px(132.))
            .rounded(colors.radius)
            .border_1()
            .border_color(colors.border)
            .bg(colors.panel_bg)
            .shadow_md()
            .p_1()
            .flex()
            .flex_col()
            .gap_0p5()
            // `items` 为 Copy 数组，用 `iter().copied()` 按值传递而不消费，content 闭包（多次调用）可安全复用。
            .children(items.iter().copied().map(|item| make_item(item, popover.clone())))
    })
}

/// 把结构化回复渲染成可读文本。
fn redis_workbench_reply_row(reply: &CommandReply, colors: UiColors) -> Div {
    let (color, label) = match reply {
        CommandReply::Error(msg) => (rgb(0xff5c5c), msg.clone()),
        CommandReply::Nil => (colors.muted, "(nil)".to_string()),
        _ => (colors.text, command_reply_display(reply)),
    };
    div()
        .flex_1()
        .min_w(px(0.))
        .pl_5()
        .text_size(px(12.))
        .font_family(crate::PLATFORM_MONOSPACE_FONT)
        .text_color(color)
        .whitespace_normal()
        .child(label)
}

/// CommandReply → redis-cli / RedisInsight 风格的展示文本。
///
/// 顶层回复按类型展开：数组递归出多行带序号的文本，整数显示 `(integer) N`、
/// nil 显示 `(nil)`、bulk 与状态直接展示原文本；映射仿 RedisInsight 把键值平铺为数组。
/// 数组元素的字符串一律加引号、嵌套数组按层级缩进，空数组显示 `(empty list or set)`。
/// 该函数同时作为结果区复制内容的唯一来源，保证「所见即所复」。
fn command_reply_display(reply: &CommandReply) -> String {
    match reply {
        CommandReply::Nil => "(nil)".to_string(),
        CommandReply::Status(s) => s.clone(),
        CommandReply::Integer(i) => format!("(integer) {i}"),
        CommandReply::Float(f) => f.to_string(),
        CommandReply::Bulk(bulk) => command_bulk_display(bulk),
        CommandReply::Array(items) => format_redis_array(items, 0),
        CommandReply::Map(pairs) => format_redis_array(&flatten_map(pairs), 0),
        CommandReply::Error(msg) => msg.clone(),
        CommandReply::Text(text) => text.clone(),
    }
}

/// 把键值映射平铺为「键、值、键、值…」列表（RedisInsight 对对象同理拍平）。
fn flatten_map(pairs: &[(CommandReply, CommandReply)]) -> Vec<CommandReply> {
    let mut flat = Vec::with_capacity(pairs.len() * 2);
    for (key, value) in pairs {
        flat.push(key.clone());
        flat.push(value.clone());
    }
    flat
}

/// 递归生成 redis-cli 风格多行数组文本。
///
/// - 空数组返回 `(empty list or set)`。
/// - 每个元素一行，带 `N)` 序号；`level` 为缩进层级（每层 3 空格），
///   仅非首个元素缩进，嵌套数组以 `level+1` 继续展开。
fn format_redis_array(items: &[CommandReply], level: usize) -> String {
    if items.is_empty() {
        return "(empty list or set)".to_string();
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let left_margin = if index > 0 { "   ".repeat(level) } else { String::new() };
            let value = format_redis_element(item, level + 1);
            format!("{left_margin}{}) {value}", index + 1)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 数组内单个元素的展示：嵌套数组 / 映射继续递归，其余按 redis-cli 语义带引号展开。
fn format_redis_element(reply: &CommandReply, level: usize) -> String {
    match reply {
        CommandReply::Nil => "(nil)".to_string(),
        CommandReply::Status(s) => format!("\"{s}\""),
        CommandReply::Integer(i) => format!("(integer) {i}"),
        CommandReply::Float(f) => format!("\"{f}\""),
        CommandReply::Bulk(bulk) => format!("\"{}\"", command_bulk_display(bulk)),
        CommandReply::Array(items) => format_redis_array(items, level),
        CommandReply::Map(pairs) => format_redis_array(&flatten_map(pairs), level),
        CommandReply::Error(msg) => format!("\"{msg}\""),
        CommandReply::Text(text) => format!("\"{text}\""),
    }
}

/// CommandBulk → 展示文本（优先 UTF-8，否则 hex 预览）。
fn command_bulk_display(bulk: &CommandBulk) -> String {
    if let Some(text) = bulk.text.as_ref() {
        text.clone()
    } else if let Some(hex) = bulk.bytes_preview_hex.as_ref() {
        format!("<{} bytes> {}", bulk.byte_len, hex)
    } else {
        format!("<{} bytes>", bulk.byte_len)
    }
}

fn query_output_panel(
    tab_id: TabId,
    editor: &QueryEditorState,
    placement: ResultsPlacement,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let has_output = !editor.results.is_empty() || !editor.summaries.is_empty();
    let has_error = editor.error.is_some();
    if !has_output && !editor.running && !has_error {
        return div();
    }

    if (has_output || has_error) && this.collapsed_query_outputs.contains(&tab_id) {
        return collapsed_query_output_button(tab_id, placement, colors, cx);
    }

    let database_kind = this.query_editor_database_kind(tab_id);
    let selected = this
        .query_output_tabs
        .get(&tab_id)
        .copied()
        .filter(|tab| query_output_tab_exists(*tab, editor, database_kind))
        .unwrap_or_else(|| {
            if query_result_entry_count(editor) > 0 {
                QueryOutputTab::Result(0)
            } else {
                QueryOutputTab::Summary
            }
        });
    let size = match placement {
        ResultsPlacement::Bottom => query_output_height(tab_id, this, window),
        ResultsPlacement::Right => query_output_width(tab_id, this, window),
    };
    let body = if let Some(error) = editor.error.as_ref() {
        let retry = Button::new(("retry-query-output", tab_id.0))
            .label(tr("app.common.retry"))
            .small()
            .outline()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.start_query_execution(tab_id, window, cx);
            }))
            .into_any_element();
        query_output_error_state(tab_id, error, Some(retry), cx, colors).into_any_element()
    } else if !has_output {
        query_output_empty_state(colors).into_any_element()
    } else {
        match selected {
            QueryOutputTab::Result(index) => query_result_view(
                tab_id,
                editor,
                index,
                this,
                window,
                colors,
                cx,
            )
            .into_any_element(),
            QueryOutputTab::Summary => {
                query_execution_summary_view(tab_id, &editor.summaries, colors, cx)
                    .into_any_element()
            }
            QueryOutputTab::Plan => {
                query_plan_view(tab_id, editor, this, colors, cx).into_any_element()
            }
        }
    };

    let content = div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .child(query_output_tabs(
            tab_id,
            editor,
            database_kind,
            selected,
            colors,
            cx,
        ))
        .child(
            div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(body)
                .when(editor.running, |this| this.child(query_output_loading_overlay(colors))),
        );
    let panel = div()
        .flex_none()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .flex();
    match placement {
        ResultsPlacement::Bottom => panel
            .h(px(size))
            .border_t_1()
            .flex_col()
            .child(query_output_resize_handle(tab_id, placement, size, colors, cx))
            .child(content),
        ResultsPlacement::Right => panel
            .w(px(size))
            .h_full()
            .border_l_1()
            .child(query_output_resize_handle(tab_id, placement, size, colors, cx))
            .child(content),
    }
}

fn query_output_loading_overlay(colors: UiColors) -> Div {
    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .occlude()
        .bg(if colors.is_dark {
            hsla(216. / 360., 0.16, 0.10, 0.76)
        } else {
            hsla(210. / 360., 0.20, 0.98, 0.72)
        })
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(loading_spinner_with_color(18., colors.muted))
        .child(tr("app.query.running"))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
}

/// SQL 结果区可用高度估算：视口高度扣除顶部工具条。
fn query_split_height(window: &Window) -> f32 {
    (f32::from(window.viewport_size().height) - QUERY_TOOLBAR_HEIGHT).max(0.)
}

/// 结果区高度：默认占分栏 `RESULT_DEFAULT_SPLIT_RATIO`，
/// 再夹到 [split/6, 0.80·split]（min 相对编辑器 20%，max 占分栏 80%）。
fn query_output_height(tab_id: TabId, this: &NavicatMain, window: &Window) -> f32 {
    let split = query_split_height(window);
    let default_height = split * RESULT_DEFAULT_SPLIT_RATIO;
    let raw = this.query_output_heights.get(&tab_id).copied().unwrap_or(default_height);
    query_output_clamp_height(raw, split)
}
fn query_output_width(tab_id: TabId, this: &NavicatMain, window: &Window) -> f32 {
    let max_width = query_output_max_width(window);
    this.query_output_widths
        .get(&tab_id)
        .copied()
        .unwrap_or(QUERY_OUTPUT_DEFAULT_WIDTH)
        .clamp(QUERY_OUTPUT_MIN_WIDTH, max_width)
}

fn query_output_max_width(window: &Window) -> f32 {
    (f32::from(window.viewport_size().width) - SQL_EDITOR_MIN_WIDTH).max(QUERY_OUTPUT_MIN_WIDTH)
}

fn query_output_resize_handle(
    tab_id: TabId,
    placement: ResultsPlacement,
    size: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let handle = div()
        .id("query-output-resize-handle")
        .flex_none()
        .bg(colors.panel_bg)
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.query_output_resize_start = Some(QueryOutputResizeStart {
                    tab_id,
                    placement,
                    x: f32::from(event.position.x),
                    y: f32::from(event.position.y),
                    size,
                });
                cx.stop_propagation();
            }),
        )
        .on_drag(QueryOutputResizeDrag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_drag_move(cx.listener(
            move |this, event: &DragMoveEvent<QueryOutputResizeDrag>, window, cx| {
                let Some(start) = this.query_output_resize_start else {
                    return;
                };
                if start.tab_id != tab_id {
                    return;
                }
                match start.placement {
                    ResultsPlacement::Bottom => {
                        let delta = start.y - f32::from(event.event.position.y);
                        let split = query_split_height(window);
                        let height = query_output_clamp_height(start.size + delta, split);
                        this.query_output_heights.insert(tab_id, height);
                    }
                    ResultsPlacement::Right => {
                        let delta = start.x - f32::from(event.event.position.x);
                        let width = (start.size + delta)
                            .clamp(QUERY_OUTPUT_MIN_WIDTH, query_output_max_width(window));
                        this.query_output_widths.insert(tab_id, width);
                    }
                }
                cx.notify();
                cx.stop_propagation();
            },
        ))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                if this
                    .query_output_resize_start
                    .is_some_and(|start| start.tab_id == tab_id)
                {
                    this.query_output_resize_start = None;
                    cx.stop_propagation();
                }
            }),
        );
    match placement {
        ResultsPlacement::Bottom => handle.h(px(7.)).cursor_ns_resize(),
        ResultsPlacement::Right => handle.w(px(7.)).h_full().cursor_col_resize(),
    }
}

fn collapsed_query_output_button(
    tab_id: TabId,
    placement: ResultsPlacement,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let container = div().flex_none().relative();
    let button = div()
        .absolute()
        .right(px(18.))
        .bottom(px(18.))
        .h(px(42.))
        .px_5()
        .rounded_full()
        .cursor_pointer()
        .bg(if colors.is_dark { rgb(0x141414) } else { rgb(0xffffff) })
        .border_1()
        .border_color(colors.border)
        .shadow_lg()
        .flex()
        .items_center()
        .gap_2()
        .text_color(colors.text)
        .hover(move |style| style.bg(colors.hover))
        .child(app_icon(AppIcon::ChevronUp, 16., colors.text))
        .child(
            div()
                .text_size(px(15.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(tr("app.query.show_results")),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.collapsed_query_outputs.remove(&tab_id);
                cx.notify();
                cx.stop_propagation();
            }),
        );
    match placement {
        ResultsPlacement::Bottom => container.h(px(0.)).child(button),
        ResultsPlacement::Right => container.w(px(0.)).h_full().child(button),
    }
}

fn query_output_tab_exists(
    tab: QueryOutputTab,
    editor: &QueryEditorState,
    database_kind: DatabaseKind,
) -> bool {
    match tab {
        QueryOutputTab::Result(index) => index < query_result_entry_count(editor),
        QueryOutputTab::Summary => !editor.summaries.is_empty() || editor.error.is_some(),
        // 只有当前结果能解析成计划时才出现；否则宁可回退到原始结果表格。
        QueryOutputTab::Plan => editor
            .results
            .last()
            .is_some_and(|page| fluxdb_app::query_plan_from_page(database_kind, page).is_some()),
    }
}

fn data_cell_edit_should_commit_before_query_output_tab_change(
    current: Option<DataCellEditState>,
    tab_id: TabId,
    selected: bool,
) -> bool {
    !selected && current.is_some_and(|editing| editing.tab_id == tab_id)
}

fn query_output_tabs(
    tab_id: TabId,
    editor: &QueryEditorState,
    database_kind: DatabaseKind,
    selected: QueryOutputTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let refresh_sql = selected
        .result_index()
        .and_then(|index| query_result_sql(editor, index));
    let tabs = div()
        .h(px(34.))
        .flex_none()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .gap_2()
        .px_3();

    // 结果页签可能很多：页签区单独横向滚动，右侧操作按钮固定可见。
    let mut tab_strip = div()
        .id(("query-output-tab-strip", tab_id.0))
        .h_full()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .items_center()
        .gap_2()
        .overflow_x_scroll();

    for index in 0..query_result_entry_count(editor) {
        tab_strip = tab_strip.child(
            query_output_tab_button(
                tab_id,
                QueryOutputTab::Result(index),
                tr("app.query.result_tab").replace("%{index}", &(index + 1).to_string()),
                None,
                selected == QueryOutputTab::Result(index),
                colors,
                cx,
            )
            .flex_none(),
        );
    }

    tab_strip = tab_strip.child(
        query_output_tab_button(
            tab_id,
            QueryOutputTab::Summary,
            tr("app.query.summary_tab").to_string(),
            Some(AppIcon::List),
            selected == QueryOutputTab::Summary,
            colors,
            cx,
        )
        .flex_none(),
    );

    if query_output_tab_exists(QueryOutputTab::Plan, editor, database_kind) {
        tab_strip = tab_strip.child(
            query_output_tab_button(
                tab_id,
                QueryOutputTab::Plan,
                tr("app.query.plan.tab").to_string(),
                Some(AppIcon::FileSearch),
                selected == QueryOutputTab::Plan,
                colors,
                cx,
            )
            .flex_none(),
        );
    }

    let refresh = query_output_toolbar_button(
        tab_id,
        tr("app.common.refresh"),
        AppIcon::Refresh,
        colors,
        cx,
        move |this, tab_id, window, cx| {
            if let Some(sql) = refresh_sql.clone() {
                this.start_query_text_execution(tab_id, sql, window, cx);
            } else {
                this.start_query_execution(tab_id, window, cx);
            }
        },
    )
    .flex_none();
    let collapse = query_output_toolbar_button(
        tab_id,
        tr("app.query.collapse_results"),
        AppIcon::ChevronDown,
        colors,
        cx,
        |this, tab_id, _, cx| {
            this.collapsed_query_outputs.insert(tab_id);
            cx.notify();
        },
    )
    .flex_none();

    tabs.child(tab_strip)
        .when(
            selected
                .result_index()
                .and_then(|index| query_result_page_index(editor, index))
                .is_some_and(|page_index| editor.result_editors.contains_key(&page_index)),
            |this| this.child(div().flex_none().child(query_result_editable_badge(colors))),
        )
        .child(refresh)
        .child(collapse)
}

fn query_result_editable_badge(colors: UiColors) -> Div {
    div()
        .h(px(26.))
        .px_2()
        .flex()
        .items_center()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if colors.is_dark {
            rgb(0x7ee787)
        } else {
            rgb(0x188038)
        })
        .child(tr("app.query.editable"))
}

fn query_output_tab_button(
    tab_id: TabId,
    tab: QueryOutputTab,
    label: String,
    icon: Option<AppIcon>,
    selected: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let debug_name = match tab {
        QueryOutputTab::Result(index) => format!("query-output-tab:result:{index}"),
        QueryOutputTab::Summary => "query-output-tab:summary".to_string(),
        QueryOutputTab::Plan => "query-output-tab:plan".to_string(),
    };
    div()
        .debug_selector(move || debug_name.clone())
        .h(px(26.))
        .px_2()
        .rounded(colors.radius_lg)
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .bg(if selected {
            if colors.is_dark {
                rgb(0x2a2e36)
            } else {
                rgb(0xffffff)
            }
        } else {
            colors.panel_bg
        })
        .text_color(if selected { colors.text } else { colors.muted })
        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
        .when_some(icon, |this, icon| {
            this.child(app_icon(icon, 15., if selected { colors.text } else { colors.muted }))
        })
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .whitespace_nowrap()
                .child(label),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _window, cx| {
                if selected {
                    cx.stop_propagation();
                    return;
                }
                if data_cell_edit_should_commit_before_query_output_tab_change(
                    this.data_cell_editing,
                    tab_id,
                    selected,
                ) && !this.commit_data_cell_edit_with_refresh(false, cx)
                {
                    cx.stop_propagation();
                    return;
                }
                this.query_output_tabs.insert(tab_id, tab);
                let page_index = tab.result_index().and_then(|index| {
                    this.controller
                        .state()
                        .tabs
                        .iter()
                        .find(|state_tab| state_tab.id == tab_id)
                        .and_then(|state_tab| match &state_tab.kind {
                            TabKind::QueryEditor(editor) => query_result_page_index(editor, index),
                            _ => None,
                        })
                });
                this.dispatch(
                    AppCommand::ActivateQueryResultEditor { tab_id, page_index },
                    cx,
                );
                if let Some(statement) = tab
                    .result_index()
                    .and_then(|index| query_result_statement_ordinal_for_tab(
                        this.controller.state(),
                        tab_id,
                        index,
                    ))
                    .and_then(|ordinal| {
                        // 新编辑器：依据全文切分语句并按 ordinal 取目标语句。
                        this.query_editors.get(&tab_id).and_then(|editor| {
                            let text = editor.read(cx).text();
                            sql_editor_adapter::build_statement_runs(
                                &text,
                                this.query_editor_database_kind(tab_id),
                            )
                                .get(ordinal)
                                .cloned()
                        })
                    })
                    && let Some(sql_editor) = this.query_editors.get(&tab_id).cloned()
                {
                    sql_editor.update(cx, |editor, cx| {
                        editor.reveal_offset(statement.range.start, cx);
                    });
                }
                cx.notify();
                cx.stop_propagation();
            }),
        )
}

fn query_output_toolbar_button(
    tab_id: TabId,
    label: impl Into<SharedString>,
    icon: AppIcon,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
    action: impl Fn(&mut NavicatMain, TabId, &mut Window, &mut Context<NavicatMain>) + Clone + 'static,
) -> Div {
    let label = label.into();
    div()
        .h(px(26.))
        .px_2()
        .rounded(colors.radius_lg)
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .text_color(colors.muted)
        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
        .child(app_icon(icon, 14., colors.muted))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(label),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                action(this, tab_id, window, cx);
                cx.stop_propagation();
            }),
        )
}

fn query_output_empty_state(colors: UiColors) -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .text_color(colors.muted)
        .child(tr("app.content.empty_results_hint"))
}

/// 查询 / Redis 输出面板的错误态：常驻 gpui-component `Alert`（不参与自动消失），
/// 顶部对齐放在面板里；`retry` 由调用方自备（面板自带运行按钮时可以不传）。
fn query_output_error_state(
    tab_id: TabId,
    error: &fluxdb_core::UserFacingError,
    retry: Option<gpui::AnyElement>,
    cx: &App,
    colors: UiColors,
) -> Div {
    div().flex_1().min_h(px(0.)).child(
        div().overflow_y_scrollbar().p_6().child(page_error_alert(
            gpui::ElementId::Name(format!("query-output-error-{}", tab_id.0).into()),
            &error.title,
            &error.message,
            error.detail.as_deref(),
            retry,
            cx,
            colors,
        )),
    )
}
/// 空结果集的列头条：仅对列列表渲染列名，供 0 行但保留列头的结果显示。
fn query_result_empty_columns_header(
    columns: &[fluxdb_core::Column],
    colors: UiColors,
) -> impl IntoElement {
    div()
        .w_full()
        .h(px(32.))
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .text_size(px(13.))
        .text_color(colors.muted)
        .children(columns.iter().enumerate().map(|(index, column)| {
            let name = if column.name.is_empty() {
                tr("app.content.column_n").replace("%{index}", &(index + 1).to_string())
            } else {
                column.name.clone()
            };
            div()
                .flex_none()
                .max_w(px(240.))
                .overflow_hidden()
                .whitespace_nowrap()
                .child(name)
        }))
}


#[derive(Clone, Copy)]
enum QueryResultEntry<'a> {
    Page {
        page: &'a DataPage,
        page_index: usize,
    },
    Failure {
        summary: &'a QueryExecutionSummary,
    },
}

fn query_result_summary_has_result_tab(summary: &QueryExecutionSummary) -> bool {
    summary.kind == fluxdb_core::QueryStatementKind::ResultSet || !summary.success
}

fn query_result_entry_count(editor: &QueryEditorState) -> usize {
    query_execution_result_entry_count(editor.results.len(), &editor.summaries)
}

fn query_execution_result_entry_count(
    result_count: usize,
    summaries: &[QueryExecutionSummary],
) -> usize {
    if summaries.is_empty() {
        return result_count;
    }

    summaries
        .iter()
        .filter(|summary| query_result_summary_has_result_tab(summary))
        .count()
        .max(result_count)
}

fn query_result_entry<'a>(
    editor: &'a QueryEditorState,
    result_index: usize,
) -> Option<QueryResultEntry<'a>> {
    if editor.summaries.is_empty() {
        return editor.results.get(result_index).map(|page| QueryResultEntry::Page {
            page,
            page_index: result_index,
        });
    }

    let mut entry_index = 0usize;
    let mut page_index = 0usize;
    for summary in &editor.summaries {
        if !query_result_summary_has_result_tab(summary) {
            continue;
        }

        if !summary.success {
            if entry_index == result_index {
                return Some(QueryResultEntry::Failure { summary });
            }
            entry_index += 1;
            continue;
        }

        let Some(page) = editor.results.get(page_index) else {
            continue;
        };
        if entry_index == result_index {
            return Some(QueryResultEntry::Page { page, page_index });
        }
        entry_index += 1;
        page_index += 1;
    }

    editor.results.get(page_index).map(|page| QueryResultEntry::Page {
        page,
        page_index,
    })
}

fn query_result_page_index(editor: &QueryEditorState, result_index: usize) -> Option<usize> {
    match query_result_entry(editor, result_index)? {
        QueryResultEntry::Page { page_index, .. } => Some(page_index),
        QueryResultEntry::Failure { .. } => None,
    }
}

fn query_result_summary_index(editor: &QueryEditorState, result_index: usize) -> Option<usize> {
    if editor.summaries.is_empty() {
        return Some(result_index);
    }

    let mut entry_index = 0usize;
    for (summary_index, summary) in editor.summaries.iter().enumerate() {
        if !query_result_summary_has_result_tab(summary) {
            continue;
        }
        if entry_index == result_index {
            return Some(summary_index);
        }
        entry_index += 1;
    }
    None
}

fn query_result_statement_ordinal_for_tab(
    state: &AppState,
    tab_id: TabId,
    result_index: usize,
) -> Option<usize> {
    state
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .and_then(|tab| match &tab.kind {
            TabKind::QueryEditor(editor) => query_result_summary_index(editor, result_index),
            _ => None,
        })
}

fn query_result_sql(editor: &QueryEditorState, result_index: usize) -> Option<String> {
    if editor.summaries.is_empty() {
        return None;
    }

    let mut entry_index = 0usize;
    for summary in &editor.summaries {
        if !query_result_summary_has_result_tab(summary) {
            continue;
        }

        if entry_index == result_index {
            return Some(summary.sql.clone());
        }
        entry_index += 1;
    }

    None
}

fn query_result_view(
    tab_id: TabId,
    editor: &QueryEditorState,
    result_index: usize,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(entry) = query_result_entry(editor, result_index) else {
        return query_output_empty_state(colors);
    };
    let (page, page_index) = match entry {
        QueryResultEntry::Page { page, page_index } => (page, page_index),
        QueryResultEntry::Failure { summary } => {
            return query_result_error_state(tab_id, result_index, summary, this, colors, cx);
        }
    };
    let result_editor = editor.result_editors.get(&page_index);
    let result_page = result_editor
        .and_then(|editor| editor.page.as_ref())
        .unwrap_or(page);
    let sort_rules = this
        .query_result_sort_rules
        .get(&QueryResultSortKey {
            tab_id,
            result_index,
        })
        .cloned()
        .unwrap_or_default();
    let sorted_page =
        this.query_result_display_page(tab_id, result_index, page_index, result_page, &sort_rules, false);
    let page = &sorted_page.page;
    let search_open = this.data_search_panels.contains(&tab_id);
    if search_open {
        let expected = this
            .data_search_queries
            .get(&tab_id)
            .cloned()
            .unwrap_or_default();
        let input_focused = this
            .data_search_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if !input_focused && this.data_search_input.read(cx).value().to_string() != expected {
            this.data_search_input.update(cx, |input, cx| {
                input.set_value(expected, window, cx);
            });
        }
    }
    if page.rows.is_empty() {
        // 空结果仍保留列头（T13 后端按列头生成），避免 PG/MY 空集合查询丢字段信息。
        let column_header = if page.columns.is_empty() {
            None
        } else {
            Some(
                query_result_empty_columns_header(&page.columns, colors)
                    .into_any_element(),
            )
        };
        return div()
            .size_full()
            .flex()
            .flex_col()
            .child(column_header.unwrap_or_else(|| div().into_any_element()))
            .child(
                div()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .h(px(48.))
                            .flex()
                            .items_center()
                            .px_3()
                            .text_size(px(13.))
                            .text_color(colors.muted)
                            .child(tr("app.content.no_rows")),
                    ),
            );
    }

    let readonly_detail = result_editor
        .is_none()
        .then(|| this.query_result_cell_detail.get(&tab_id).copied())
        .flatten();
    let editable_detail_open = result_editor.is_some_and(|editor| editor.cell_detail_panel.open);
    let detail_height = if readonly_detail
        .is_some_and(|detail| query_result_cell_detail_exists(result_page, detail))
        || editable_detail_open
    {
            this.cell_detail_drawer_heights
                .get(&tab_id)
                .copied()
                .map(clamp_cell_detail_drawer_height)
                .unwrap_or(CELL_DETAIL_DRAWER_DEFAULT_HEIGHT)
    } else {
        0.
    };
    let readonly_panel = readonly_detail.and_then(|detail| {
        query_result_cell_detail_panel(
            tab_id,
            result_page,
            detail,
            detail_height,
            colors,
            this,
            cx,
        )
    });
    let table_state = this.query_result_table_state_from_sorted_page(
        tab_id,
        sorted_page.page.clone(),
        page_index,
        &sort_rules,
        &sorted_page.source_row_indexes,
        result_editor,
        window,
        cx,
    );
    let (search_matches, active_search_match) = {
        let table = table_state.read(cx);
        let delegate = table.delegate();
        (delegate.search_matches.clone(), delegate.active_search_match)
    };
    let search_highlight_all = this.data_search_highlight_all_tabs.contains(&tab_id);
    let change_count = result_editor
        .and_then(|editor| editor.changes.as_ref())
        .filter(|changes| !changes.is_empty())
        .map(data_change_item_count);
    let change_sql_preview = result_editor
        .and_then(|_| change_count)
        .and_then(|_| result_editor.and_then(|editor| editor.changes.as_ref()))
        .map(|changes| {
            let db_kind = this.connection_database_kind(changes.object.connection_id);
            data_change_sql_preview(result_page, changes, db_kind)
        });
    let change_sql_preview_open = this.data_change_sql_preview_tabs.contains(&tab_id);
    let large_value_window = result_editor
        .filter(|editor| editor.cell_detail_panel.open)
        .and_then(|editor| {
            let cell = editor.cell_detail_panel.active_cell?;
            let large = result_page
                .rows
                .get(cell.row)?
                .values
                .get(cell.column)?
                .large_value()?
                .clone();
            Some(this.ensure_large_value_window(tab_id, cell.row, cell.column, &large, cx))
        });
    if large_value_window.is_none() && readonly_detail.is_none() {
        this.drop_large_value_window(tab_id);
    }
    let cell_detail_drawer = result_editor
        .filter(|editor| editor.cell_detail_panel.open)
        .map(|editor| {
            let input = this.cell_detail_input.clone();
            sync_input_value(&input, &editor.cell_detail_panel.edit_value, window, cx);
            cell_detail_panel(
                tab_id,
                editor,
                result_page,
                large_value_window.as_ref(),
                detail_height,
                input,
                this.temporal_part_input.clone(),
                this.temporal_part_editing,
                colors,
                window,
                cx,
            )
        });
    let search_height = if search_open { 30. } else { 0. };
    let change_footer_height = if change_count.is_some() { 30. } else { 0. };
    let sql_preview_height = if change_sql_preview_open && change_sql_preview.is_some() {
        160.
    } else {
        0.
    };
    let page_footer_height = if page.offset > 0 || page.has_more || page.incomplete.is_some() {
        36.
    } else {
        0.
    };
    let data_search_input = this.data_search_input.clone();
    let data_page_input = this.data_page_input.clone();
    let pagination_sql = query_result_sql(editor, result_index).unwrap_or_default();
    div()
        .relative()
        .size_full()
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .top_0()
                .right_0()
                .bottom(px(
                    detail_height
                        + change_footer_height
                        + sql_preview_height
                        + page_footer_height
                        + search_height,
                ))
                .left_0()
                .overflow_hidden()
                .child(component_data_table(&table_state, cx)),
        )
        .when(search_open, |this| {
            this.child(
                data_search_bar(
                    tab_id,
                    data_search_input,
                    search_matches.as_slice(),
                    active_search_match,
                    search_highlight_all,
                    colors,
                    cx,
                )
                .absolute()
                .right_0()
                .bottom(px(
                    detail_height + change_footer_height + sql_preview_height + page_footer_height,
                ))
                .left_0(),
            )
        })
        .when_some(change_sql_preview.as_ref(), |this, preview| {
            this.when(change_sql_preview_open, |this| {
                this.child(
                    data_change_sql_preview_drawer(tab_id, preview, colors, cx)
                        .absolute()
                        .right_0()
                        .bottom(px(if detail_height > 0. { detail_height + 30. } else { 30. }))
                        .left_0()
                        .h(px(160.)),
                )
            })
        })
        .when_some(readonly_panel, |this, panel| {
            this.child(panel.absolute().right_0().bottom_0().left_0())
        })
        .when_some(cell_detail_drawer, |this, panel| {
            this.child(panel.absolute().right_0().bottom_0().left_0().h(px(detail_height)))
        })
        .when_some(change_count, |this, count| {
            this.child(
                query_result_change_footer(
                    tab_id,
                    count,
                    change_sql_preview_open,
                    editor.running,
                    colors,
                    cx,
                )
                    .absolute()
                    .right_0()
                    .bottom(px(detail_height))
                    .left_0(),
            )
        })
        .when(page_footer_height > 0., |this| {
            this.child(
                query_result_page_footer(
                    tab_id,
                    result_index,
                    page_index,
                    page,
                    pagination_sql,
                    data_page_input,
                    colors,
                    window,
                    cx,
                )
                .absolute()
                .right_0()
                .bottom(px(detail_height + change_footer_height + sql_preview_height))
                .left_0(),
            )
        })
}

fn query_result_page_footer(
    tab_id: TabId,
    result_index: usize,
    page_index: usize,
    page: &DataPage,
    sql: String,
    page_input: Entity<InputState>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let page_size = data_page_limit(page.limit);
    let page_no = data_page_number(page.offset, page_size);
    let can_previous = page.offset > 0;
    // 资源不完整时同样可以继续，但不能按配置页大小越过未展示的行。
    let can_next = page.has_more || page.incomplete.is_some();
    let previous_offset = page.offset.saturating_sub(page_size);
    let next_offset = page.next_offset();

    div()
        .h(px(36.))
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .px_3()
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .when_some(page.incomplete.as_ref(), |this, incomplete| {
            this.child(
                div()
                    .id("query-result-incomplete")
                    .debug_selector(|| "query-result-incomplete".into())
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(colors.warning)
                    .child(
                        tr("app.pagination.incomplete")
                            .replace("%{reason}", incomplete.reason.trim()),
                    ),
            )
        })
        .child(
            query_result_page_button(
                AppIcon::ChevronsLeft,
                tr("app.pagination.first"),
                can_previous,
                colors,
            )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let sql = sql.clone();
                        move |this, _, window, cx| {
                            if can_previous {
                                this.sync_data_page_input_value(0, page_size, window, cx);
                                this.start_query_result_page_refresh(
                                    QueryResultRefreshRequest {
                                        tab_id,
                                        result_index,
                                        page_index,
                                        sql: sql.clone(),
                                        offset: 0,
                                        limit: page_size,
                                    },
                                    cx,
                                );
                            }
                            cx.stop_propagation();
                        }
                    }),
                ),
        )
        .child(
            query_result_page_button(
                AppIcon::ChevronLeft,
                tr("app.pagination.previous"),
                can_previous,
                colors,
            )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let sql = sql.clone();
                        move |this, _, window, cx| {
                            if can_previous {
                                this.sync_data_page_input_value(
                                    previous_offset,
                                    page_size,
                                    window,
                                    cx,
                                );
                                this.start_query_result_page_refresh(
                                    QueryResultRefreshRequest {
                                        tab_id,
                                        result_index,
                                        page_index,
                                        sql: sql.clone(),
                                        offset: previous_offset,
                                        limit: page_size,
                                    },
                                    cx,
                                );
                            }
                            cx.stop_propagation();
                        }
                    }),
                ),
        )
        .child(data_editor_page_input(
            page_no, page_input, colors, window, cx,
        ))
        .child(
            query_result_page_button(
                AppIcon::ChevronRight,
                tr("app.pagination.next"),
                can_next,
                colors,
            )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let sql = sql.clone();
                        move |this, _, window, cx| {
                            if can_next {
                                this.sync_data_page_input_value(next_offset, page_size, window, cx);
                                this.start_query_result_page_refresh(
                                    QueryResultRefreshRequest {
                                        tab_id,
                                        result_index,
                                        page_index,
                                        sql: sql.clone(),
                                        offset: next_offset,
                                        limit: page_size,
                                    },
                                    cx,
                                );
                            }
                            cx.stop_propagation();
                        }
                    }),
                ),
        )
        .child(
            query_result_page_button(
                AppIcon::ChevronsRight,
                tr("app.pagination.last_unknown"),
                false,
                colors,
            )
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
        )
}

fn query_result_page_button(
    icon: AppIcon,
    tooltip: impl Into<SharedString>,
    enabled: bool,
    colors: UiColors,
) -> Stateful<Div> {
    let tooltip = tooltip.into();
    let tooltip_for_id = tooltip.clone();
    div()
        .id(tooltip_for_id)
        .size(px(24.))
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .opacity(if enabled { 1.0 } else { 0.45 })
        .hover(move |style| {
            if enabled {
                style.bg(colors.hover)
            } else {
                style
            }
        })
        .when(enabled, |this| this.cursor_pointer())
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(app_icon(icon, 15., if enabled { colors.text } else { colors.muted }))
}

fn query_result_error_state(
    tab_id: TabId,
    result_index: usize,
    summary: &QueryExecutionSummary,
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let copy_text = query_result_error_copy_text(summary);
    let summary_for_ai = summary.clone();
    let ai_busy = this.ai_chat.loading || this.ai_chat.running.is_some()
        || this.ai_chat.pending_sql_edit.is_some();
    let (variant, icon_name, accent) = app_message_style(AppMessageKind::Error, cx);
    let alert = Alert::new(
        gpui::ElementId::Name(
            format!("query-result-error-{}-{}", tab_id.0, result_index).into(),
        ),
        localize_backend_error_text(&summary.message),
    )
    .with_variant(variant)
    .icon(alert_icon(icon_name, cx, PAGE_ERROR_LINE_HEIGHT))
    .title(tr("app.query.error_title"))
    .bg(alert_tint(accent, colors))
    .line_height(px(PAGE_ERROR_LINE_HEIGHT));

    let sql_block = div()
        .w_full()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(30.))
                .px_3()
                .border_b_1()
                .border_color(colors.border)
                .flex()
                .items_center()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(tr("app.query.failed_sql")),
        )
        .child(
            div()
                .p_3()
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_size(px(12.))
                .line_height(px(18.))
                .text_color(colors.text)
                .whitespace_normal()
                .child(summary.sql.clone()),
        );

    let actions = div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            query_error_action_button(
                gpui::ElementId::Name(
                    format!("retry-query-result-{}-{}", tab_id.0, result_index).into(),
                ),
                tr("app.common.retry"),
                AppIcon::Refresh,
                colors,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.start_query_execution(tab_id, window, cx);
            })),
        )
        .child(
            query_error_action_button(
                gpui::ElementId::Name(
                    format!("copy-query-result-error-{}-{}", tab_id.0, result_index).into(),
                ),
                tr("app.query.copy_error"),
                AppIcon::Copy,
                colors,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                this.show_message(
                    tr("app.query.error_copied"),
                    AppMessageKind::Success,
                    cx,
                );
            })),
        )
        .child(
            query_error_action_button(
                gpui::ElementId::Name(
                    format!("ai-fix-query-result-error-{}-{}", tab_id.0, result_index).into(),
                ),
                tr("app.query.fix_with_ai"),
                AppIcon::Bot,
                colors,
            )
            .disabled(ai_busy)
            .debug_selector(|| "query-ai-fix".into())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.begin_ai_sql_editor_edit(
                    tab_id,
                    Some(QueryFailureRepair {
                        result_index,
                        summary: summary_for_ai.clone(),
                    }),
                    window,
                    cx,
                );
            })),
        );

    div().flex_1().min_h(px(0.)).child(
        div().overflow_y_scrollbar().child(
            div()
                .w_full()
                .flex()
                .justify_center()
                .p_6()
                .child(
                    div()
                        .w_full()
                        .max_w(px(880.))
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(alert)
                        .child(sql_block)
                        .child(actions),
                ),
        ),
    )
}

fn query_error_action_button(
    id: gpui::ElementId,
    label: impl Into<SharedString>,
    icon: AppIcon,
    colors: UiColors,
) -> Button {
    let label = label.into();
    Button::new(id)
        .small()
        .outline()
        .cursor_pointer()
        .w(px(108.))
        .child(app_icon(icon, 13., colors.muted))
        .child(label)
}

fn query_result_error_copy_text(summary: &QueryExecutionSummary) -> String {
    format!(
        "{}\n{}\n\n{}:\n{}",
        tr("app.query.sql_label"),
        summary.sql.trim(),
        tr("app.query.error_label"),
        summary.message.trim()
    )
}

fn query_result_cell_detail_exists(page: &DataPage, detail: QueryResultCellDetail) -> bool {
    page.rows
        .get(detail.row)
        .and_then(|row| row.values.get(detail.column))
        .is_some()
        && page.columns.get(detail.column).is_some()
}

fn query_result_change_footer(
    tab_id: TabId,
    change_count: usize,
    change_sql_preview_open: bool,
    submitting: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(30.))
        .w_full()
        .flex_none()
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .pl_1()
        .pr_2()
        .flex()
        .items_center()
        .gap_1()
        .child(
            data_editor_footer_icon_button(
                AppIcon::Check,
                tr("app.confirm.apply_changes_title"),
                !submitting,
                colors,
            )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.request_apply_data_changes(tab_id, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(
            data_editor_footer_icon_button(
                AppIcon::Close,
                tr("app.content.cancel_changes"),
                !submitting,
                colors,
            )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.dispatch(AppCommand::DiscardDataChanges(tab_id), cx);
                        this.data_change_sql_preview_tabs.remove(&tab_id);
                        this.refresh_active_data_table(tab_id, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(data_editor_change_status(
            tab_id,
            change_count,
            change_sql_preview_open,
            submitting,
            colors,
            cx,
        ))
}

fn query_result_cell_detail_panel(
    tab_id: TabId,
    page: &DataPage,
    detail: QueryResultCellDetail,
    height: f32,
    colors: UiColors,
    this: &mut NavicatMain,
    cx: &mut Context<NavicatMain>,
) -> Option<Div> {
    let column = page.columns.get(detail.column)?;
    let value = page.rows.get(detail.row)?.values.get(detail.column)?.clone();
    let large = value.large_value().cloned();
    // 大字段没有「复制整值」入口：不取标签文本，省掉一次（可能有界的）整串复制。
    let value_text = large.is_none().then(|| cell_value_label(&value));
    let type_name = column
        .type_name
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let type_color = data_type_color(&type_name).unwrap_or(colors.text);
    let drawer_bg = if colors.is_dark {
        rgb(0x171b21)
    } else {
        rgb(0xfbfcfe)
    };
    let card_bg = if colors.is_dark {
        rgb(0x20252d)
    } else {
        rgb(0xffffff)
    };
    let soft_border = if colors.is_dark {
        rgb(0x2f3642)
    } else {
        colors.border_soft
    };
    // 大字段窗口先就绪：标题行与正文都按同一份窗口状态渲染。
    let large_window = large.as_ref().map(|large| {
        this.ensure_large_value_window(tab_id, detail.row, detail.column, large, cx)
    });

    Some(
        div()
            .h(px(height))
            .w_full()
            .flex_none()
            .border_t_1()
            .border_color(soft_border)
            .bg(drawer_bg)
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(cell_detail_drawer_resize_handle(
                tab_id,
                height,
                colors,
                cx,
            ))
            .child(
                div()
                    .h(px(54.))
                    .w_full()
                    .rounded(colors.radius * 0.5)
                    .border_1()
                    .border_color(soft_border)
                    .bg(card_bg)
                    .flex()
                    .overflow_hidden()
                    .child(cell_detail_meta_item(
                        tr("app.content.column_name"),
                        column.name.clone(),
                        colors,
                    ))
                    .child(cell_detail_meta_item(
                        tr("app.content.row_number"),
                        (detail.row + 1).to_string(),
                        colors,
                    ))
                    .child(cell_detail_meta_item_colored(
                        tr("app.history.meta_type"),
                        type_name,
                        type_color,
                        colors,
                    ))
                    .child(cell_detail_meta_item(
                        "NULL",
                        matches!(value, CellValue::Null).to_string(),
                        colors,
                    ))
                    .child(cell_detail_meta_item(
                        tr("app.content.length"),
                        cell_value_length(&value).to_string(),
                        colors,
                    ))
                    .child(
                        div()
                            .w(px(42.))
                            .h_full()
                            .flex_none()
                            .flex()
                            .items_start()
                            .justify_center()
                            .pt_2()
                            .child(
                                cell_detail_icon_button(
                                    AppIcon::Close,
                                    tr("app.common.close"),
                                    colors,
                                )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.query_result_cell_detail.remove(&tab_id);
                                            this.large_value_windows.remove(&tab_id);
                                            cx.notify();
                                            cx.stop_propagation();
                                        }),
                                    ),
                            ),
                    ),
            )
            .child(query_result_cell_detail_value_header(
                tab_id,
                value_text,
                large_window.as_ref(),
                colors,
                cx,
            ))
            .child(match large_window.as_ref() {
                // 只读结果里的大字段同样只给分段窗口；不预读，点「读取更多」才占配额。
                Some(window) => large_value_window_view(Some(window), colors),
                None => cell_detail_value_preview(tab_id, &value, colors, cx),
            }),
    )
}

fn query_result_cell_detail_value_header(
    tab_id: TabId,
    value_text: Option<String>,
    large_window: Option<&LargeValueWindows>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let status = large_value_status(large_window, colors);
    div()
        .h(px(22.))
        .w_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            // 左组必须让出空间：状态里的错误文案很长时，按钮不能被挤出详情面板。
            div()
                .flex()
                .flex_1()
                .items_center()
                .gap_2()
                .min_w(px(0.))
        .child(
            div()
                        .flex_none()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(tr("app.placeholder.value")),
        )
                .when_some(status, |this, status| this.child(status)),
        )
        .child(if large_window.is_some() {
            // 操作按钮与可编辑结果一致放在标题行；大字段只给「下载」与「读取更多」。
            // 未读完就保留按钮：读取中换成转圈，不让按钮凭空消失。
            let can_load_more = large_window.is_some_and(|window| !window.view.is_complete());
            let load_more_loading = large_window.is_some_and(|window| window.view.loading);
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    cell_detail_icon_button(
                        AppIcon::Download,
                        tr("app.cell_detail.large_download_tip"),
                        colors,
                    )
                    .debug_selector(|| "large-value-save".into())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.save_large_value_to_file(tab_id, cx);
                            cx.stop_propagation();
                        }),
                    ),
                )
                .when(can_load_more, |this| {
                    if load_more_loading {
                        // 读取中只转圈、不响应点击，尺寸与其它图标按钮一致。
                        this.child(
                            cell_detail_icon_spinner_button(
                                tr("app.cell_detail.large_loading"),
                                colors,
                            )
                            .debug_selector(|| "large-value-load-more".into()),
                        )
                    } else {
                        this.child(
                            cell_detail_icon_button(
                                AppIcon::ChevronsDown,
                                tr("app.cell_detail.large_load_more_tip"),
                                colors,
                            )
                            .debug_selector(|| "large-value-load-more".into())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.load_next_large_value_chunk(tab_id, cx);
                                    cx.notify();
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                    }
                })
                .into_any_element()
        } else {
            let copy_text = value_text.unwrap_or_default();
            cell_detail_icon_button(AppIcon::Copy, tr("app.context.copy"), colors).on_mouse_down(
            MouseButton::Left,
            move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                cx.stop_propagation();
            },
            )
            .into_any_element()
        })
}

#[derive(Clone)]
struct SortedQueryResultPage {
    /// 与表格委托共享，避免每次刷新再复制一份完整结果页。
    page: Arc<DataPage>,
    /// 同样共享：渲染时克隆缓存页不再按行数复制索引表。
    source_row_indexes: Arc<[usize]>,
}

fn sorted_query_result_page(page: &DataPage, rules: &[DataSortRule]) -> SortedQueryResultPage {
    let active = rules
        .iter()
        .filter(|rule| rule.enabled)
        .filter_map(|rule| {
            page.columns
                .iter()
                .position(|column| column.name == rule.field)
                .map(|index| (index, rule.ascending))
        })
        .collect::<Vec<_>>();
    if active.is_empty() {
        return SortedQueryResultPage {
            page: Arc::new(page.clone()),
            source_row_indexes: (0..page.rows.len()).collect(),
        };
    }

    let mut rows = page
        .rows
        .iter()
        .cloned()
        .enumerate()
        .collect::<Vec<_>>();
    rows.sort_by(|(_, left), (_, right)| {
        for (index, ascending) in &active {
            let ordering =
                compare_query_result_cells(left.values.get(*index), right.values.get(*index));
            if ordering != std::cmp::Ordering::Equal {
                return if *ascending { ordering } else { ordering.reverse() };
            }
        }
        std::cmp::Ordering::Equal
    });
    let (source_row_indexes, rows): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
    SortedQueryResultPage {
        page: Arc::new(DataPage {
            rows,
            ..page.clone()
        }),
        source_row_indexes: source_row_indexes.into(),
    }
}

fn compare_query_result_cells(
    left: Option<&CellValue>,
    right: Option<&CellValue>,
) -> std::cmp::Ordering {
    match (left, right) {
        (Some(CellValue::Null), Some(CellValue::Null)) => std::cmp::Ordering::Equal,
        (Some(CellValue::Null), _) => std::cmp::Ordering::Less,
        (_, Some(CellValue::Null)) => std::cmp::Ordering::Greater,
        (Some(CellValue::I64(left)), Some(CellValue::I64(right))) => left.cmp(right),
        (Some(CellValue::F64(left)), Some(CellValue::F64(right))) => {
            left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
        }
        (Some(left), Some(right)) => left.display_label().cmp(&right.display_label()),
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
    }
}

fn query_execution_summary_view(
    tab_id: TabId,
    summaries: &[QueryExecutionSummary],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut table = div()
        .id(("query-execution-summary-table", tab_id.0))
        .debug_selector(|| "query-summary-table".into())
        .w_full()
        .min_w(px(QUERY_SUMMARY_TABLE_MIN_WIDTH))
        .flex()
        .flex_col();
    table = table.child(query_summary_header(colors));
    for (row_index, summary) in summaries.iter().enumerate() {
        table = table.child(query_summary_row(row_index, summary, colors, cx));
    }
    if summaries.is_empty() {
        table = table.child(
            div()
                .h(px(48.))
                .flex()
                .items_center()
                .px_3()
                .text_size(px(13.))
                .text_color(colors.muted)
                .child(tr("app.query.no_summary")),
        );
    }
    div()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .flex()
        .flex_col()
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .id(("query-execution-summary-scroll", tab_id.0))
                .debug_selector(|| "query-summary-scroll".into())
                // `overflow_scrollbar()` 把本元素当作滚动区里的 flex 子项，默认交叉轴
                // 拉伸会让表格高度恒等于视口，纵向 scroll_max 为 0、行多了只被裁掉。
                // `self_start` 取消拉伸，高度回到内容高度，纵向滚动才成立。
                .self_start()
                .overflow_scrollbar()
                .child(table),
        )
}

#[derive(Clone, Copy)]
enum QuerySummaryColumn {
    Statement,
    Type,
    Returned,
    Affected,
    Elapsed,
}

impl QuerySummaryColumn {
    fn debug_name(self) -> &'static str {
        match self {
            Self::Statement => "statement",
            Self::Type => "type",
            Self::Returned => "returned",
            Self::Affected => "affected",
            Self::Elapsed => "elapsed",
        }
    }

    fn apply(self, cell: Div) -> Div {
        let cell = match self {
            Self::Statement => cell
                .flex_1()
                .flex_basis(px(0.))
                .min_w(px(QUERY_SUMMARY_STATEMENT_MIN_WIDTH)),
            Self::Type => cell.flex_none().w(px(QUERY_SUMMARY_TYPE_WIDTH)),
            Self::Returned => cell.flex_none().w(px(QUERY_SUMMARY_RETURNED_WIDTH)),
            Self::Affected => cell.flex_none().w(px(QUERY_SUMMARY_AFFECTED_WIDTH)),
            Self::Elapsed => cell.flex_none().w(px(QUERY_SUMMARY_ELAPSED_WIDTH)),
        };
        match self {
            Self::Statement => cell.pl_2().pr_1(),
            Self::Type => cell.px_2(),
            Self::Returned | Self::Affected | Self::Elapsed => cell.px_2(),
        }
    }

    fn is_numeric(self) -> bool {
        matches!(self, Self::Returned | Self::Affected | Self::Elapsed)
    }
}

fn query_summary_header(colors: UiColors) -> Div {
    div()
        .h(px(34.))
        .flex_none()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .child(query_summary_header_cell(
            QuerySummaryColumn::Statement,
            tr("app.query.header.statement"),
            colors,
        ))
        .child(query_summary_header_cell(
            QuerySummaryColumn::Type,
            tr("app.query.header.type"),
            colors,
        ))
        .child(query_summary_header_cell(
            QuerySummaryColumn::Returned,
            tr("app.query.header.returned"),
            colors,
        ))
        .child(query_summary_header_cell(
            QuerySummaryColumn::Affected,
            tr("app.query.header.affected"),
            colors,
        ))
        .child(query_summary_header_cell(
            QuerySummaryColumn::Elapsed,
            tr("app.query.header.elapsed"),
            colors,
        ))
}

fn query_summary_header_cell(
    column: QuerySummaryColumn,
    text: impl Into<String>,
    colors: UiColors,
) -> Div {
    let column_name = column.debug_name();
    let text = single_line_summary_text(text.into());
    column
        .apply(
            div()
                .debug_selector(move || format!("query-summary-header:{column_name}"))
                .h_full()
                .flex()
                .items_center()
                .overflow_hidden(),
        )
        .when(column.is_numeric(), |this| this.justify_end())
        .child(
            div()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(text),
        )
}

fn query_summary_row(
    row_index: usize,
    summary: &QueryExecutionSummary,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(40.))
        .flex_none()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(colors.border_soft)
        .text_color(colors.text)
        .child(query_summary_sql_cell(row_index, summary, colors, cx))
        .child(query_summary_type_cell(row_index, summary, colors, cx))
        .child(query_summary_numeric_cell(
            row_index,
            QuerySummaryColumn::Returned,
            summary.returned_rows.to_string(),
            colors,
        ))
        .child(query_summary_numeric_cell(
            row_index,
            QuerySummaryColumn::Affected,
            summary.affected_rows.to_string(),
            colors,
        ))
        .child(query_summary_numeric_cell(
            row_index,
            QuerySummaryColumn::Elapsed,
            format!("{}ms", summary.elapsed_ms),
            colors,
        ))
}

fn query_summary_sql_cell(
    row_index: usize,
    summary: &QueryExecutionSummary,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let sql_text = single_line_summary_text(summary.sql.clone());
    QuerySummaryColumn::Statement
        .apply(
            div()
                .debug_selector(move || format!("query-summary-row:{row_index}:statement"))
                .h_full()
                .flex()
                .items_center()
                .gap_1()
                .overflow_hidden(),
        )
        .child(
            div()
                .id(("query-summary-sql", row_index))
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.))
                .text_color(colors.text)
                .tooltip(query_summary_full_text_tooltip(summary.sql.clone(), true))
                .child(sql_text),
        )
        .child(query_summary_copy_button(
            ("copy-query-summary-sql", row_index),
            tr("app.query.copy_sql"),
            summary.sql.clone(),
            tr("app.query.sql_copied"),
            colors,
            cx,
        ))
}

fn query_summary_copy_button(
    id: (&'static str, usize),
    tooltip: impl Into<SharedString>,
    copy_text: String,
    success_text: impl Into<SharedString>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let tooltip = tooltip.into();
    let success_text = success_text.into();
    div()
        .id(id)
        .size(px(26.))
        .rounded(colors.radius)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(app_icon(AppIcon::Copy, 14., colors.muted))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                this.show_message(success_text.clone(), AppMessageKind::Success, cx);
                cx.stop_propagation();
            }),
        )
}

/// 悬停展示完整文本；SQL 用等宽字体，错误报文按普通文本自动换行。
fn query_summary_full_text_tooltip(
    text: String,
    monospace: bool,
) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    move |window, cx| {
        let text = text.clone();
        Tooltip::element(move |_, _| {
            div()
                .max_w(px(560.))
                .whitespace_normal()
                .text_size(px(12.))
                .when(monospace, |this| {
                    this.font_family(crate::PLATFORM_MONOSPACE_FONT)
                })
                .child(text.clone())
        })
        .build(window, cx)
    }
}

fn query_summary_type_cell(
    row_index: usize,
    summary: &QueryExecutionSummary,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let badge_bg = if summary.success {
        if colors.is_dark {
            rgb(0x053f31)
        } else {
            rgb(0xdff8ec)
        }
    } else if colors.is_dark {
        rgb(0x552126)
    } else {
        rgb(0xffe4e6)
    };
    let badge_text = if summary.success {
        rgb(0x20c76a)
    } else {
        rgb(0xff5c5c)
    };
    QuerySummaryColumn::Type
        .apply(
            div()
                .debug_selector(move || format!("query-summary-row:{row_index}:type"))
                .h_full()
                .flex()
                .items_center()
                .gap_2()
                .overflow_hidden(),
        )
        .child(
            div()
                .px_2()
                .h(px(20.))
                .rounded_full()
                .bg(badge_bg)
                .flex()
                .items_center()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(badge_text)
                .child(if summary.success {
                    tr("app.history.success")
                } else {
                    tr("app.history.failed")
                }),
        )
        .child(
            div()
                .id(("query-summary-message", row_index))
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.))
                .text_color(colors.text)
                .tooltip(query_summary_full_text_tooltip(summary.message.clone(), false))
                .child(localize_backend_error_text(&summary.message)),
        )
        .when(!summary.success, |this| {
            this.child(query_summary_copy_button(
                ("copy-query-summary-error", row_index),
                tr("app.query.copy_error"),
                summary.message.clone(),
                tr("app.query.error_copied"),
                colors,
                cx,
            ))
        })
}

fn query_summary_numeric_cell(
    row_index: usize,
    column: QuerySummaryColumn,
    text: impl Into<String>,
    colors: UiColors,
) -> Div {
    let text = single_line_summary_text(text.into());
    let column_name = column.debug_name();
    column
        .apply(
            div()
                .debug_selector(move || format!("query-summary-row:{row_index}:{column_name}"))
                .h_full()
                .flex()
                .items_center()
                .justify_end()
                .overflow_hidden(),
        )
        .child(
            div()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.))
                .text_color(colors.text)
                .child(text),
        )
}

fn single_line_summary_text(text: String) -> String {
    if !text.contains('\n') && !text.contains('\r') {
        return text;
    }

    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

include!("query_toolbar.rs");

fn object_toolbar(colors: UiColors) -> impl IntoElement {
    div()
        .h(px(36.))
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .justify_between()
        .px_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(17.))
                .text_color(rgb(0x1687ff))
                .child(app_icon_box(AppIcon::Square, 22., 17., rgb(0x1687ff)))
                .child(app_icon_box(AppIcon::Edit, 22., 17., rgb(0x1687ff)))
                .child(app_icon_box(AppIcon::Plus, 22., 17., rgb(0x1687ff)))
                .child(app_icon_box(AppIcon::Minus, 22., 17., rgb(0x1687ff)))
                .child(app_icon_box(AppIcon::Undo, 22., 17., rgb(0x1687ff)))
                .child(app_icon_box(AppIcon::Redo, 22., 17., rgb(0x1687ff)))
                .child(app_icon_box(AppIcon::Refresh, 22., 17., rgb(0x1687ff))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(app_icon_box(AppIcon::List, 22., 18., rgb(0x1687ff)))
                .child(app_icon_box(AppIcon::Table, 22., 18., colors.muted))
                .child(
                    div()
                        .w(px(180.))
                        .h(px(28.))
                        .rounded(colors.radius_lg)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.input_bg)
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child(app_icon(AppIcon::Search, 13., colors.muted))
                        .child(tr("app.placeholder.search")),
                ),
        )
}

/// 展示模式双态 segmented control（平铺 / Folder）：位于左侧列表顶部状态栏最左侧，纯图标无文字。
/// 未选中段使用普通底 + hover；选中段填充高亮色（亮/暗主题均取 UiColors 适配色）。
/// 展示仅用 lucide 风格语义图标：`AppIcon::List`（平铺）/ `AppIcon::Folder`（Folder），不出现文字。
fn redis_key_mode_segmented(
    tab_id: TabId,
    mode: RedisKeyListMode,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id(("redis-key-mode-switch", tab_id.0))
        .h(px(32.))
        .w(px(104.))
        .flex_none()
        .p(px(3.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .gap(px(3.))
        .child(
            redis_key_mode_segment(
                tab_id,
                "flat",
                RedisKeyListMode::Flat,
                AppIcon::List,
                mode,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.set_redis_key_list_mode(tab_id, RedisKeyListMode::Flat, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            redis_key_mode_segment(
                tab_id,
                "folder",
                RedisKeyListMode::Folder,
                AppIcon::Folder,
                mode,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.set_redis_key_list_mode(tab_id, RedisKeyListMode::Folder, cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

fn redis_key_mode_segment(
    tab_id: TabId,
    tag: &'static str,
    seg_mode: RedisKeyListMode,
    icon: AppIcon,
    current: RedisKeyListMode,
    colors: UiColors,
) -> Stateful<Div> {
    let active = current == seg_mode;
    // 选中段高亮：亮暗主题共用品牌蓝，白色图标保证可读性。
    let accent: Hsla = rgb(0x0c5fd5).into();
    div()
        .id(SharedString::from(format!(
            "redis-key-mode-seg-{}-{}",
            tab_id.0, tag
        )))
        .flex_1()
        .h_full()
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(active, |this| this.bg(accent))
        .when(!active, |this| this.hover(move |style| style.bg(colors.hover)))
        .child(app_icon(
            icon,
            14.,
            if active { rgb(0xffffff) } else { colors.muted },
        ))
}

fn redis_key_search_bar(
    tab_id: TabId,
    type_select: Entity<SelectState<SearchableVec<RedisTypeFilterOption>>>,
    input: Entity<InputState>,
    active: bool,
    refreshing: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 搜索栏（`relative`，高 40px）作为顶层历史下拉重建前缀的参照：历史下拉不再作为本节点的子节点
    // 渲染，而是移到 `redis_split_view` 之后（见 render 根）以保证绘制在表格之上。
    // 展示模式双态切换、结果数 / 已扫描 / Scan more 已迁移到左侧列表顶部状态栏（`redis_key_list_status_bar`）。
    div()
        .relative()
        .h(px(40.))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .child(redis_search_type_select_box(type_select))
        .child(redis_search_input_frame(tab_id, input, colors, cx))
        .when(active, |this| {
            this.child(
                div()
                    .ml_4()
                    .text_size(px(13.))
                    .text_color(colors.text)
                    .child(tr("app.content.last_search_now")),
            )
        })
        .child(div().flex_1().min_w(px(0.)))
        .child(redis_refresh_key_list_button(refreshing, colors).when(!refreshing, |this| {
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.request_data_editor_refresh(tab_id, cx);
                    cx.stop_propagation();
                }),
            )
        }))
        .child(redis_add_key_button(colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.open_redis_add_key_drawer(tab_id, window, cx);
                cx.stop_propagation();
            }),
        ))
}

fn redis_search_type_select_box(
    select: Entity<SelectState<SearchableVec<RedisTypeFilterOption>>>,
) -> Div {
    div()
        .h(px(34.))
        .w(px(142.))
        .flex_none()
        .child(
            Select::new(&select)
                .small()
                .title_prefix(tr("app.content.type_prefix"))
                .w_full()
                .h_full()
                .menu_width(px(132.)),
        )
}

fn redis_search_input_frame(
    tab_id: TabId,
    input: Entity<InputState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let has_text = !input.read(cx).value().is_empty();
    // 输入框宽度规则（`Fraction(0.5)`/`min_w(360)`）被顶层历史下拉锚点复用，
    // 故本处不受影响、可正常保持 `relative`；历史下拉实际渲染逻辑见 `redis_key_search_history_overlay`。
    // 不能加 `overflow_hidden`，否则绝对定位的下拉会被裁剪在输入框高度内无法展开；
    // 输入框圆角裁剪由内层 `rounded_lg` + 各子元素自带裁剪保证。
    div()
        .relative()
        .h(px(34.))
        .w(gpui::DefiniteLength::Fraction(0.5))
        .min_w(px(360.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .child(
            Input::new(&input)
                .appearance(false)
                .focus_bordered(false)
                .flex_1()
                .min_w(px(0.))
                .w_full()
                .h_full()
                .px_2()
                .text_size(px(13.)),
        )
        .when(has_text, |this| {
            this.child(redis_search_icon_button(
                AppIcon::Close,
                tr("app.history.clear"),
                colors,
            ).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.clear_redis_search(tab_id, window, cx);
                    cx.stop_propagation();
                }),
            ))
        })
        .child(
            redis_search_icon_button(
                AppIcon::ArrowUpDown,
                tr("app.content.search_history"),
                colors,
            ).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.toggle_redis_key_search_history(tab_id, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(redis_search_icon_button(
            AppIcon::Search,
            tr("app.placeholder.search"),
            colors,
        ).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.apply_redis_search(tab_id, cx);
                cx.stop_propagation();
            }),
        ))
}

fn redis_refresh_key_list_button(refreshing: bool, colors: UiColors) -> Stateful<Div> {
    div()
        .id("redis-refresh-keys")
        .h(px(34.))
        .px_3()
        .flex_none()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .gap_2()
        .when(!refreshing, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .tooltip(|window, cx| {
                    Tooltip::new(tr("app.common.refresh")).build(window, cx)
                })
        })
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if refreshing { colors.muted } else { colors.text })
        .child(if refreshing {
            loading_spinner_with_color(14., colors.muted).into_any_element()
        } else {
            app_icon(AppIcon::Refresh, 14., colors.muted).into_any_element()
        })
        .child(tr("app.common.refresh"))
}

fn redis_add_key_button(colors: UiColors) -> Stateful<Div> {
    div()
        .id("redis-add-key")
        .h(px(34.))
        .px_3()
        .flex_none()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .tooltip(|window, cx| {
            Tooltip::new(tr("app.content.new_key")).build(window, cx)
        })
        .child(app_icon(AppIcon::Plus, 14., rgb(0x1687ff)))
        .child(tr("app.content.new_key"))
}

fn redis_search_icon_button(
    icon: AppIcon,
    tooltip: impl Into<SharedString>,
    colors: UiColors,
) -> Stateful<Div> {
    let tooltip = tooltip.into();
    div()
        .id(tooltip.clone())
        .size(px(26.))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(app_icon(icon, 14., colors.muted))
}

/// 搜索历史下拉的顶层锚点：挂到 `redis_split_view` 之后渲染，保证下拉浮于表格之上
/// （gpui 无 z-index，兄弟节点按绘制顺序叠放，后置者在上）。
/// 这里重建搜索栏前缀（px_2 + 类型选择 142 + gap + 输入框 `Fraction(0.5)`/`min_w(360)`），
/// 复用 taffy 对输入框宽度的精确计算，使下拉左边缘对齐输入框左边缘、宽度严格等于输入框。
/// 该锚点自身与搜索栏区域重叠但不含任何鼠标监听，故不会拦截下方搜索栏的交互。
fn redis_key_search_history_overlay(
    tab_id: TabId,
    history: Vec<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 仅占搜索栏高度一行；下拉为绝对定位，会从该行下方展开，故此处无需 `overflow_hidden`。
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(40.))
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        // 类型选择框占位，保持与 `redis_key_search_bar` 一致的横向偏移
        // （展示模式 segmented control 已迁到左侧列表顶部状态栏，不再占用搜索栏宽度）。
        .child(div().w(px(142.)).flex_none())
        .child(
            // 输入框宽度锚点：与 `redis_search_input_frame` 相同的宽度规则。
            div()
                .relative()
                .h(px(34.))
                .w(gpui::DefiniteLength::Fraction(0.5))
                .min_w(px(360.))
                .child(redis_key_search_history_dropdown(tab_id, history, colors, cx)),
        )
}

/// Key 搜索历史下拉菜单：锚定在输入框（`relative`）正下方，左边缘对齐输入框左边缘并撑满输入框宽度，
/// 对齐 RedisInsight 中下拉归属于搜索组件的做法。历史词点击即回填并应用，「清除历史」按当前连接 + DB 清空。
/// 外部点击关闭由 `redis_key_search_history_occlude` 全屏遮罩层负责，故菜单自身拦截点击避免穿透。
fn redis_key_search_history_dropdown(
    tab_id: TabId,
    history: Vec<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let menu = if history.is_empty() {
        // 空历史：仅展示空态提示，不渲染清除按钮。
        div()
            .h(px(36.))
            .px_3()
            .flex()
            .items_center()
            .text_size(px(13.))
            .text_color(colors.muted)
            .child(tr("app.content.no_search_history"))
    } else {
        let mut list = div().flex().flex_col();
        for text in history.iter() {
            let item_text = text.clone();
            list = list.child(
                div()
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(move |style| style.bg(colors.hover))
                    .text_size(px(13.))
                    .text_color(colors.text)
                    .child(app_icon(AppIcon::Search, 14., colors.muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(item_text.clone()),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.apply_redis_key_search_history(
                                tab_id,
                                item_text.clone(),
                                window,
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    ),
            );
        }
        div().max_h(px(280.)).child(list.overflow_y_scrollbar())
    };

    let dropdown = div()
        .absolute()
        .top(px(38.))
        // 左边缘对齐搜索输入框左边缘；宽度取输入框实际宽度的 100%（`w_full` = 定位包含块即输入框的宽度），
        // 使下拉紧贴输入框正下方、宽度严格等于输入框；不使用 `right` 侧锚定，避免跑到右侧空白区域。
        .left(px(0.))
        .w_full()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow_lg()
        .flex()
        .flex_col()
        .overflow_hidden()
        // 标题行
        .child(
            div()
                .h(px(32.))
                .px_3()
                .flex()
                .items_center()
                .border_b_1()
                .border_color(colors.border_soft)
                .bg(colors.panel_alt)
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(tr("app.content.search_history")),
        )
        .child(menu)
        .when(!history.is_empty(), |this| {
            // 清除历史：按当前连接 + DB 清空
            this.child(
                div()
                    .h(px(32.))
                    .border_t_1()
                    .border_color(colors.border_soft)
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(move |style| style.bg(colors.hover))
                    .text_size(px(13.))
                    .text_color(rgb(0xe5484d))
                    .child(app_icon(AppIcon::Trash, 14., rgb(0xe5484d)))
                    .child(tr("app.content.clear_history"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.clear_redis_key_search_history(tab_id);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
        });

    // 菜单自身拦截滚轮/移动/左键，避免事件穿透到下层全屏遮罩或表格。
    dropdown
        .on_mouse_move(|_, _, cx| {
            cx.stop_propagation();
        })
        .on_scroll_wheel(|_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
}

/// 全屏遮罩层：捕捉搜索历史下拉之外的外部点击用于关闭，避免与菜单点击冲突。
fn redis_key_search_history_occlude(colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    let _ = colors;
    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .occlude()
        .on_mouse_move(|_, _, cx| {
            cx.stop_propagation();
        })
        .on_scroll_wheel(|_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.close_redis_key_search_history(cx);
                cx.stop_propagation();
            }),
        )
        .on_mouse_down(MouseButton::Right, |_, _, cx| {
            cx.stop_propagation();
        })
}

fn data_editor_toolbar(
    tab_id: TabId,
    filter_open: bool,
    filter_mode: DataFilterMode,
    database_kind: DatabaseKind,
    search_open: bool,
    field_filter_open: bool,
    field_count: Option<(usize, usize)>,
    table_info_open: Option<bool>,
    show_table_tools: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let filter_dialect = sql_editor_adapter::SqlDialect::from_database_kind(database_kind);
    div()
        .h(px(40.))
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .justify_between()
        .px_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .when(show_table_tools, |this| this.child(
                    div()
                        .debug_selector(|| "data-filter-sort-toggle".into())
                        .h(px(28.))
                        .px_3()
                        .rounded(colors.radius_lg)
                        .border_1()
                        .border_color(if filter_open {
                            rgb(0x7db4ff)
                        } else {
                            colors.border
                        })
                        .bg(if filter_open {
                            if colors.is_dark {
                                rgb(0x16345f)
                            } else {
                                rgb(0xe8f2ff)
                            }
                        } else {
                            colors.panel_alt
                        })
                        .text_size(px(13.))
                        .text_color(if filter_open {
                            rgb(0x1677ff)
                        } else {
                            colors.text
                        })
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap_2()
                        .hover(move |style| style.bg(colors.hover))
                        .child(app_icon(
                            AppIcon::Filter,
                            15.,
                            if filter_open {
                                rgb(0x1677ff)
                            } else {
                                colors.text
                            },
                        ))
                        .child(tr("app.data.toolbar.filter_sort"))
                        .child(app_icon(
                            if filter_open {
                                AppIcon::ChevronUp
                            } else {
                                AppIcon::ChevronDown
                            },
                            13.,
                            if filter_open {
                                rgb(0x1677ff)
                            } else {
                                colors.muted
                            },
                        ))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                this.toggle_data_filter_panel(tab_id, window, cx);
                                cx.stop_propagation();
                            }),
                        ),
                ))
                .when(filter_open, |this| {
                    this.child(data_filter_toolbar_mode_switch(
                        tab_id,
                        filter_mode,
                        colors,
                        filter_dialect,
                        cx,
                    ))
                })
                .when(show_table_tools, |this| this.child(
                    data_editor_action_state(tr("app.data.toolbar.search"), search_open, colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.toggle_data_search_panel(tab_id, window, cx);
                            cx.stop_propagation();
                        }),
                    ),
                ))
                .when(show_table_tools, |this| this.child(data_editor_action(tr("app.data.toolbar.analyze"), colors)))
                .when(show_table_tools, |this| this.child(
                    data_editor_action(tr("app.data.toolbar.import"), colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.open_table_import_wizard_for_tab(tab_id, window, cx);
                            cx.stop_propagation();
                        }),
                    ),
                ))
                .when(show_table_tools, |this| this.child(data_editor_action(tr("app.data.toolbar.export"), colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.open_table_data_export(tab_id, cx);
                        cx.stop_propagation();
                    }),
                )))
                .when(show_table_tools, |this| this.child(data_editor_action(tr("app.data.toolbar.generate"), colors))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .when_some(field_count, |this, (visible, total)| {
                    this.child(
                        div()
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.muted)
                            .child(if visible == total {
                                tr("app.data.field_count")
                                    .replace("%{visible}", &total.to_string())
                                    .replace("%{total}", &total.to_string())
                            } else {
                                tr("app.data.field_count_filtered")
                                    .replace("%{visible}", &visible.to_string())
                                    .replace("%{total}", &total.to_string())
                        }),
                    )
                })
                .when(show_table_tools, |this| this.child(
                    data_editor_action(tr("app.data.toolbar.field_filter"), colors)
                        .when(field_filter_open, |this| {
                            this.border_color(rgb(0x7db4ff)).bg(if colors.is_dark {
                                rgb(0x16345f)
                            } else {
                                rgb(0xe8f2ff)
                            })
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.toggle_field_filter_popover(tab_id, cx);
                                cx.stop_propagation();
                            }),
                        ),
                ))
                .when(show_table_tools, |this| this.when_some(table_info_open, |this, active| {
                    this.child(
                        data_editor_action_state(tr("app.data.toolbar.table_info"), active, colors)
                            .id(("table-info-toggle", tab_id.0))
                            .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.dispatch(
                                    AppCommand::ToggleTableInfo {
                                        tab_id,
                                        tab: TableInfoTab::Columns,
                                    },
                                    cx,
                                );
                                cx.stop_propagation();
                            }),
                        ),
                    )
                })),
        )
}

fn data_editor_action(label: impl Into<SharedString>, colors: UiColors) -> Div {
    let label = label.into();
    div()
        .h(px(28.))
        .px_3()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .text_size(px(13.))
        .text_color(colors.text)
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover))
        .child(label)
}

fn data_editor_action_state(
    label: impl Into<SharedString>,
    active: bool,
    colors: UiColors,
) -> Div {
    data_editor_action(label, colors).when(active, |this| {
        this.border_color(rgb(0x7db4ff)).bg(if colors.is_dark {
            rgb(0x16345f)
        } else {
            rgb(0xe8f2ff)
        })
    })
}

#[cfg(test)]
mod workbench_split_tests {
    use super::*;

    #[test]
    fn clamp_keeps_both_panes_above_pixel_floor() {
        // 400px 分栏：占比例下限只有 80px，必须被 100px 像素下限抬高。
        let split = 400.;
        assert_eq!(
            workbench_clamp_editor_size(0., split),
            WORKBENCH_EDITOR_MIN_HEIGHT
        );
        let max = workbench_clamp_editor_size(10_000., split);
        assert!(max >= WORKBENCH_EDITOR_MIN_HEIGHT);
        assert!(split - max >= WORKBENCH_RESULT_MIN_HEIGHT - 0.001);
    }

    #[test]
    fn clamp_falls_back_to_half_when_split_is_too_small() {
        let split = 150.;
        assert_eq!(workbench_clamp_editor_size(120., split), split / 2.);
    }
}
