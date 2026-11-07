/// Windows/Linux 自绘窗口控制。关闭按钮 hover 变红（Windows 惯例）用的常量色：
/// 浅/深主题通用的红，接近 Windows 标准关闭按钮（#E81123）。
const WINDOW_CLOSE_HOVER: gpui::Rgba = gpui::Rgba {
    r: 0.91,
    g: 0.07,
    b: 0.13,
    a: 1.0,
};

/// 顶部栏图标按钮：统一尺寸/圆角/手形光标/hover/tooltip，直接以 AppIcon 渲染。
/// 「仅图标」按钮的通用封装，用于顶部栏收起侧边栏 / 首页等无标签按钮。
///
/// `enabled` 为 false 时置灰（`opacity`，与本文件其它禁用块的观感一致）、
/// 去掉手形光标与 hover 反馈，但**保留 tooltip** —— 否则用户看到一个点不动的按钮
/// 却不知道为什么。
fn topbar_icon_button(
    icon: AppIcon,
    tooltip: impl Into<SharedString>,
    enabled: bool,
    colors: UiColors,
) -> Stateful<Div> {
    let tooltip = tooltip.into();
    div()
        .size(px(30.))
        .ml(px(2.))
        .rounded(colors.radius_lg)
        .flex()
        .items_center()
        .justify_center()
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
        })
        .when(!enabled, |this| this.opacity(0.45))
        .id(tooltip.clone())
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(app_icon_box(icon, 30., 17., colors.muted))
}

/// 顶部栏连接信息文本：超长省略 + 悬停 tooltip 展示完整连接名。
fn connection_info_label(name: String, colors: UiColors) -> Stateful<Div> {
    let tooltip_name = name.clone();
    div()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(px(13.))
        .text_color(colors.text)
        .id("topbar-connection-name")
        .tooltip(move |window, cx| Tooltip::new(tooltip_name.clone()).build(window, cx))
        .child(name)
}

/// 顶部栏：只保留原生窗口控制（红绿灯预留区）、侧边栏收起、首页与当前连接信息。
/// 展开/收起两态内容不同：
/// - 展开（show_connection_browser）：红绿灯 + 收起侧边栏 + 首页 + 连接信息。
/// - 收起：红绿灯 + 连接信息（收起/首页按钮移入收起侧边栏条）。
fn topbar(
    state: &AppState,
    show_connection_browser: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    // 当前活动标签所属连接名；无活动标签或 Settings 标签（无连接）时不展示。
    let connection_info = state.active_tab().and_then(|tab| {
        tab_workspace_scope(tab).map(|scope| connection_name(state, scope.connection_id))
    });

    div()
        .h(px(36.))
        .bg(colors.app_bg)
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .gap_1()
        // macOS 保留原生红绿灯（关闭/最小化/全屏）预留区；Windows/Linux 自绘按钮在右侧，无需预留。
        .pl(if cfg!(target_os = "macos") {
            px(84.)
        } else {
            px(12.)
        })
        .pr_3()
        // 侧边栏展开/收起切换按钮（图标随状态变化）+ 首页按钮，两态都保留在顶部栏。
        .child(
            topbar_icon_button(
                if show_connection_browser {
                    AppIcon::PanelLeftClose
                } else {
                    AppIcon::PanelLeftOpen
                },
                if show_connection_browser {
                    tr("app.topbar.collapse_sidebar")
                } else {
                    tr("app.topbar.expand_sidebar")
                },
                true,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.show_connection_browser = !this.show_connection_browser;
                    cx.stop_propagation();
                    cx.notify();
                }),
            ),
        )
        .child(
            topbar_icon_button(AppIcon::Home, tr("app.topbar.home"), true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.dispatch(AppCommand::DeactivateTab, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .when_some(connection_info, |this, name| {
            // 当前打开的数据库连接信息（文本），与活动表/库保持关联。
            this.child(
                div()
                    .flex_none()
                    .max_w(px(300.))
                    .h(px(28.))
                    .px_3()
                    .rounded(colors.radius_lg)
                    .bg(colors.panel_bg)
                    .border_1()
                    .border_color(colors.border)
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(app_icon_box(AppIcon::Database, 26., 15., colors.muted))
                    .child(connection_info_label(name, colors)),
            )
        })
        .child(
            div()
                // Windows 将 WindowControlArea::Drag 映射为 HTCAPTION，交由系统处理移动和双击；
                // macOS/Linux 由 gpui 的 start_window_move 交给平台窗口管理器。
                // 下按仅由 macOS/Linux 消费，Windows 交给 DefWindowProc，避免拦截原生拖拽。
                .id("topbar-window-drag")
                .flex_1()
                .h_full()
                .when(cfg!(target_os = "windows"), |this| {
                    this.window_control_area(WindowControlArea::Drag)
                })
                .when(!cfg!(target_os = "windows"), |this| {
                    this.on_mouse_down(MouseButton::Left, |event, window, cx| {
                        if event.click_count >= 2 {
                            toggle_window_maximized(window);
                        } else {
                            window.start_window_move();
                        }
                        cx.stop_propagation();
                    })
                }),
        )
        // 顶部栏最右侧：github + 询问 AI + 历史 + 设置。
        // 「询问 AI」原先挂在 SQL 编辑器工具栏上，移到这里与其余全局入口同排；
        // 「历史」「设置」是业务按钮移除后收纳在此的既有项。
        .child(
            topbar_icon_button(AppIcon::Github, tr("app.topbar.github"), true, colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|_, _, _, cx| {
                        cx.open_url(GITHUB_REPOSITORY_URL);
                        cx.stop_propagation();
                    }),
                ),
        )
        // 全局入口：关闭/重新打开不会重放任务。
        .child(
            Button::new("ask-ai")
                .debug_selector(|| "ask-ai".into())
                .small()
                .ghost()
                .cursor_pointer()
                .child(app_icon(AppIcon::Bot, 16., colors.muted))
                .tooltip(tr("app.topbar.ask_ai"))
                .on_click(cx.listener(|this, _, window, cx| this.open_ai_chat(window, cx))),
        )
        .child(
            topbar_icon_button(
                AppIcon::CalendarClock,
                tr("app.topbar.history"),
                true,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.toggle_history(window, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            topbar_icon_button(AppIcon::Settings, tr("app.topbar.settings"), true, colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.dispatch(AppCommand::OpenSettings, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
        // 非 macOS：GPUI 隐藏原生标题栏后不绘制窗口控制按钮，需由应用自绘
        // 最小化/最大化(还原)/关闭，否则 Windows/Linux 右上角无法关闭窗口。
        // macOS 走原生红绿灯（topbar 顶部 `pl(84.)` 预留区），不渲染。
        .when(!cfg!(target_os = "macos"), |this| {
            this.child(window_control_buttons(window, colors, cx))
        })
}

/// Windows/Linux 自绘窗口控制按钮（最小化 / 最大化-还原 / 关闭），
/// 因为隐藏原生标题栏后 GPUI 不提供窗口按钮，需应用自绘；macOS 走原生红绿灯不加此处。
/// 关闭按钮按 Windows 惯例 hover 变红；最大化按钮按当前最大化态切换图标。
/// 仅 `#[cfg(not(target_os = "macos"))]` 下被 topbar 引用。
fn toggle_window_maximized(window: &Window) {
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::RawWindowHandle;
        use windows::Win32::{
            Foundation::HWND,
            UI::WindowsAndMessaging::{SW_MAXIMIZE, SW_RESTORE, ShowWindowAsync},
        };

        let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
            tracing::error!(target: "fluxdb_desktop", "无法取得 Windows 窗口句柄");
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            tracing::error!(target: "fluxdb_desktop", "当前窗口不是 Win32 窗口");
            return;
        };
        let command = if window.is_maximized() {
            SW_RESTORE
        } else {
            SW_MAXIMIZE
        };
        // GPUI 0.3.3 的 Windows zoom() 只会最大化，无法还原，因此显式切换 Win32 状态。
        if !unsafe { ShowWindowAsync(HWND(handle.hwnd.get()), command) }.as_bool() {
            tracing::error!(
                target: "fluxdb_desktop",
                error = %std::io::Error::last_os_error(),
                "切换 Windows 窗口最大化状态失败"
            );
        }
    }

    #[cfg(not(target_os = "windows"))]
    window.zoom_window();
}

fn window_control_buttons(
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    // UiColors 非 Copy，闭包前先取出需要的 Rgba 值。
    let hover_bg = colors.hover;
    let muted = colors.muted;
    // 最大化/还原：is_maximized 决定按钮图标，点击时按平台切换窗口状态。
    let maximized = window.is_maximized();
    let maximize_icon = if maximized {
        AppIcon::Square
    } else {
        AppIcon::Maximize
    };
    let maximize_label = if maximized {
        tr("app.window.restore")
    } else {
        tr("app.window.maximize")
    };
    let maximize_id = if maximized {
        "win-btn-restore"
    } else {
        "win-btn-maximize"
    };

    div()
        .flex()
        .items_center()
        .ml(px(4.))
        .child(
            // 最小化
            div()
                .id("win-btn-minimize")
                .size(px(34.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(hover_bg))
                .tooltip(move |window, cx| {
                    Tooltip::new(tr("app.window.minimize")).build(window, cx)
                })
                .child(app_icon_box(AppIcon::Minus, 34., 16., muted))
                .on_mouse_down(MouseButton::Left, |_, window, _| {
                    window.minimize_window();
                }),
        )
        .child(
            // 最大化 / 还原
            div()
                .id(maximize_id)
                .size(px(34.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(hover_bg))
                .tooltip(move |window, cx| Tooltip::new(maximize_label.clone()).build(window, cx))
                .child(app_icon_box(maximize_icon, 34., 16., muted))
                // 状态变化会触发窗口 resize 重绘，is_maximized 图标随之刷新。
                .on_mouse_down(MouseButton::Left, |_, window, _| {
                    toggle_window_maximized(window);
                }),
        )
        .child(
            // 关闭：仅在有未保存内容时弹确认框，确认后才退出。
            div()
                .id("win-btn-close")
                .size(px(34.))
                .ml(px(2.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                // hover 变红：Windows 关闭按钮惯例。
                .hover(move |style| style.bg(WINDOW_CLOSE_HOVER))
                .tooltip(move |window, cx| Tooltip::new(tr("app.window.close")).build(window, cx))
                .child(app_icon_box(AppIcon::Close, 34., 16., muted))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.request_quit(cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

/// Windows 退出确认弹框：点击自绘关闭按钮后二次确认（取消 / 退出）。
/// 遮罩点击与 Esc（CancelDialog）均视为取消，仅「退出」按钮执行 cx.quit()。
fn exit_confirm_modal(colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.08, 0.62)
        } else {
            opaque_grey(0.6, 0.36)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.pending_exit_confirm = false;
            cx.stop_propagation();
            cx.notify();
        }))
        .child(
            div()
                .w(px(420.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
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
                .key_context("ExitConfirmModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.pending_exit_confirm = false;
                    cx.stop_propagation();
                    cx.notify();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(48.))
                        .flex_none()
                        .px_4()
                        .flex()
                        .items_center()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(tr("app.exit.title")),
                )
                .child(
                    div()
                        .px_4()
                        .pb_4()
                        .child(tr("app.exit.message")),
                )
                .child(
                    div()
                        .h(px(52.))
                        .flex_none()
                        .px_4()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .child(
                            Button::new("exit-confirm-cancel")
                                .label(tr("app.common.cancel"))
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pending_exit_confirm = false;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("exit-confirm-quit")
                                .label(tr("app.exit.quit"))
                                .danger()
                                .small()
                                .on_click(cx.listener(|_, _, _, cx| {
                                    tracing::info!(target: "fluxdb_desktop", "用户确认退出，退出应用");
                                    cx.quit();
                                })),
                        ),
                ),
        )
}

/// 新建/编辑连接弹框：左侧类型导航 + 右侧表单（设计稿 docs/design/new-connection.html）。
fn new_connection_modal(
    kind: DatabaseKind,
    active_tab: NewConnectionTab,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    editing: bool,
    test_busy: bool,
    saving: bool,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let view = cx.entity();
    let title = if editing {
        tr("app.connection.edit_title")
    } else {
        tr("app.connection.new_title")
    };

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.08, 0.62)
        } else {
            opaque_grey(0.6, 0.36)
        })
        .flex()
        .items_center()
        .justify_center()
        // 遮罩作为模态作用域根：UI 自动化用 within 限定到弹框内，避开页面同名控件。
        .id("new-connection-modal")
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Right, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(
            div()
                .relative()
                // 与设计稿同尺寸，并按视口收窄：小窗口下不把底部按钮挤出窗外。
                .w(px(1180.).min(window.viewport_size().width - px(48.)))
                .h(px(810.).min(window.viewport_size().height - px(48.)))
                .rounded(colors.radius_lg + px(8.))
                .bg(colors.panel_bg)
                .border_1()
                .border_color(colors.border)
                .flex()
                .flex_col()
                .overflow_hidden()
                .text_color(colors.text)
                .key_context("NewConnectionModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_new_connection(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(connection_dialog_header(title, colors, view.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .flex()
                        .child(kind_rail(kind, editing, inputs, colors, cx))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .child(connection_hero(kind, form, colors))
                                .child(connection_name_row(form, inputs, colors, window, cx))
                                .child(connection_tab_strip(kind, active_tab, colors, cx))
                                .child(
                                    div()
                                        .id("new-connection-form-scroll")
                                        .flex_1()
                                        .min_h(px(0.))
                                        .overflow_scroll()
                                        .scrollbar_width(px(8.))
                                        // 与页签条留出间距，所有页签内容统一从同一基线开始。
                                        .child(div().px_5().pt_4().pb_5().child(
                                            new_connection_tab_content(
                                                active_tab, kind, form, inputs, colors, window, cx,
                                            ),
                                        )),
                                ),
                        ),
                )
                .child(connection_footer(form, saving, test_busy, colors, view)),
        )
}

/// 弹框标题栏：标题 + 关闭按钮。
fn connection_dialog_header(
    title: SharedString,
    colors: UiColors,
    view: Entity<NavicatMain>,
) -> impl IntoElement {
    div()
        .h(px(58.))
        .flex_none()
        .flex()
        .items_center()
        .px_5()
        .border_b_1()
        .border_color(colors.border)
        .child(
            div()
                .text_size(px(19.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(div().flex_1())
        .child(
            Button::new("new-connection-close")
                .label("×")
                .ghost()
                .w(px(30.))
                .h(px(30.))
                .text_size(px(20.))
                .on_click(move |_, _, cx| {
                    view.update(cx, |this, cx| this.cancel_new_connection(cx));
                    cx.stop_propagation();
                }),
        )
}

/// 左侧数据库类型导航：搜索框 + 分组列表 + 驱动数量。
fn kind_rail(
    selected: DatabaseKind,
    locked: bool,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let query = inputs.kind_search.read(cx).value().trim().to_lowercase();
    let groups: Vec<(SharedString, Vec<DatabaseKind>)> = vec![
        (
            tr("app.connection.rail.relational"),
            vec![
                DatabaseKind::MySql,
                DatabaseKind::TiDb,
                DatabaseKind::MariaDb,
                DatabaseKind::Postgres,
                DatabaseKind::Kingbase,
                DatabaseKind::SqlServer,
                DatabaseKind::Oracle,
                DatabaseKind::Dameng,
                DatabaseKind::Sqlite,
            ],
        ),
        (
            tr("app.connection.rail.nosql"),
            vec![DatabaseKind::MongoDb, DatabaseKind::Redis],
        ),
    ];

    let mut list = div()
        .id("new-connection-kind-list")
        .flex_1()
        .min_h(px(0.))
        .overflow_scroll()
        .scrollbar_width(px(6.))
        .px_2()
        .pb_3()
        .flex()
        .flex_col()
        .gap_1();
    for (group_title, kinds) in groups {
        // 搜索按展示名过滤；分组整体为空时不渲染标题，避免只剩空分组。
        let visible: Vec<DatabaseKind> = kinds
            .into_iter()
            .filter(|kind| {
                query.is_empty() || database_kind_label(*kind).to_lowercase().contains(&query)
            })
            .collect();
        if visible.is_empty() {
            continue;
        }
        list = list.child(kind_rail_group(group_title, colors));
        for kind in visible {
            list = list.child(kind_rail_row(
                database_kind_label(kind),
                kind,
                selected,
                locked,
                colors,
                cx,
            ));
        }
    }

    div()
        .w(px(258.))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .bg(colors.sidebar_bg)
        .border_r_1()
        .border_color(colors.border)
        .child(
            div().p_3().child(
                div()
                    .h(px(32.))
                    .rounded(colors.radius_lg)
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.input_bg)
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(13.))
                    .child(app_icon(AppIcon::Search, 13., colors.muted))
                    .child(
                        Input::new(&inputs.kind_search)
                            .small()
                            .appearance(false)
                            .w_full(),
                    ),
            ),
        )
        .child(list)
        .child(
            div()
                .h(px(36.))
                .flex_none()
                .px_4()
                .flex()
                .items_center()
                .gap_2()
                .border_t_1()
                .border_color(colors.border_soft)
                .child(div().size(px(6.)).rounded_full().bg(rgb(0x16a34a)))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(colors.muted)
                        .child(tr("app.connection.rail.drivers").replace("%{count}", "11")),
                ),
        )
}

fn kind_rail_group(title: SharedString, colors: UiColors) -> Div {
    div()
        .pt_2()
        .pb_1()
        .px_2()
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(colors.muted)
        .child(title)
}

fn kind_rail_row(
    label: &'static str,
    kind: DatabaseKind,
    selected: DatabaseKind,
    locked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let is_selected = kind == selected;
    let disabled = locked && !is_selected;
    // ID 按数据库类型命名而非文案，便于 UI 自动化稳定定位「切换到哪种数据库」。
    let suffix = database_kind_element_suffix(kind);
    div()
        .id(format!("new-connection-kind-{suffix}"))
        .debug_selector(move || format!("new-connection-kind-{suffix}"))
        .aria_label(label)
        .role(Role::RadioButton)
        .aria_selected(is_selected)
        .h(px(32.))
        .px_2()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(13.))
        .text_color(colors.text)
        .bg(if is_selected {
            colors.tree_selected
        } else {
            colors.sidebar_bg
        })
        .when(disabled, |this| this.opacity(0.45))
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
        })
        .child(img(database_kind_icon_path(kind)).size(px(17.)))
        .child(div().flex_1().min_w(px(0.)).truncate().child(label))
        .when(!disabled, |this| {
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.set_new_connection_kind(kind, window, cx);
                    cx.stop_propagation();
                }),
            )
        })
}

/// 数据库类型展示名（左侧导航与右侧标题共用）。
fn database_kind_label(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::MySql => "MySQL",
        DatabaseKind::TiDb => "TiDB",
        DatabaseKind::MariaDb => "MariaDB",
        DatabaseKind::Postgres => "PostgreSQL",
        DatabaseKind::Kingbase => "人大金仓 KingbaseES",
        DatabaseKind::SqlServer => "SQL Server",
        DatabaseKind::Oracle => "Oracle",
        DatabaseKind::Dameng => "达梦 DM8",
        DatabaseKind::Sqlite => "SQLite",
        DatabaseKind::MongoDb => "MongoDB",
        DatabaseKind::Redis => "Redis",
    }
}

/// 右侧内容顶部：类型图标、类型名、当前连接名与内置驱动标记。
fn connection_hero(kind: DatabaseKind, form: &NewConnectionForm, colors: UiColors) -> Div {
    let name = form.name.trim();
    div()
        .px_5()
        .pt_4()
        .pb_3()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .size(px(42.))
                .flex_none()
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_alt)
                .flex()
                .items_center()
                .justify_center()
                .child(img(database_kind_icon_path(kind)).size(px(24.))),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(px(17.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(database_kind_label(kind)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .truncate()
                        .child(if name.is_empty() { tr("app.connection.new_title") } else { SharedString::from(name.to_string()) }),
                ),
        )
        .child(
            div()
                .h(px(22.))
                .flex_none()
                .px_2()
                .rounded_full()
                .border_1()
                .border_color(colors.border_soft)
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(11.5))
                .text_color(colors.muted)
                .child(app_icon(AppIcon::Check, 12., rgb(0x16a34a)))
                .child(tr("app.connection.builtin_driver")),
        )
}

/// 连接名称 + 颜色：名称独占一行宽度，颜色固定在右侧。
fn connection_name_row(
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut swatches = div().flex().items_center().gap_2();
    for &(hex, value) in CONNECTION_COLOR_PALETTE {
        swatches = swatches.child(color_swatch_light(
            hex,
            rgb(value),
            form.color == hex,
            colors,
            cx,
        ));
    }

    div()
        .px_5()
        .pb_3()
        .flex()
        .items_end()
        .gap_4()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .child(
                    div()
                        .h(px(18.))
                        .mb_1()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(px(11.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(tr("app.connection.field.name"))
                        .child(div().text_color(colors.danger).child("*")),
                )
                .child(input_box_light(
                    ConnectionField::Name,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
        .child(
            div()
                .flex_none()
                .child(
                    div()
                        .h(px(18.))
                        .mb_1()
                        .text_size(px(11.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(tr("app.connection.field.color")),
                )
                .child(swatches),
        )
}

/// 表单页签：连接信息 / 网络与安全 / 高级（下划线样式）。
fn connection_tab_strip(
    kind: DatabaseKind,
    active_tab: NewConnectionTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut tabs = vec![
        (
            NewConnectionTab::Connection,
            tr("app.connection.tab.connection"),
        ),
        (NewConnectionTab::Tls, tr("app.connection.tab.network")),
        (
            NewConnectionTab::Advanced,
            tr("app.connection.tab.advanced"),
        ),
    ];
    // SQLite 没有 TLS/SSH 配置，隐藏空页签，避免出现“无选项”死页。
    if kind == DatabaseKind::Sqlite {
        tabs.retain(|(tab, _)| *tab != NewConnectionTab::Tls);
    }
    let mut row = div()
        .h(px(39.))
        .flex_none()
        .flex()
        .items_end()
        .gap(px(2.))
        .px_5()
        .border_b_1()
        .border_color(colors.border);
    for (index, (tab, label)) in tabs.into_iter().enumerate() {
        let selected = tab == active_tab;
        row = row.child(
            div()
                .id(format!("new-connection-tab-{index}"))
                .debug_selector(move || format!("new-connection-tab-{index}"))
                .h(px(38.))
                .px_3()
                .pb_2()
                .flex()
                .items_end()
                .text_size(px(12.5))
                .font_weight(if selected {
                    gpui::FontWeight::SEMIBOLD
                } else {
                    gpui::FontWeight::NORMAL
                })
                .text_color(if selected { rgb(0x1687ff) } else { colors.muted })
                .border_b_2()
                .border_color(if selected {
                    rgb(0x1687ff)
                } else {
                    colors.panel_bg
                })
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.set_new_connection_tab(tab, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(label),
        );
    }
    row
}

/// 底部操作区：测试状态 + 取消 / 仅保存 / 测试连接 / 保存并连接。
fn connection_footer(
    form: &NewConnectionForm,
    saving: bool,
    test_busy: bool,
    colors: UiColors,
    view: Entity<NavicatMain>,
) -> Div {
    div()
        .h(px(58.))
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .px_5()
        .border_t_1()
        .border_color(colors.border)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .items_center()
                .when_some(form.test_status.as_ref(), |this, status| {
                    this.child(connection_test_status(status))
                })
                .when_none(&form.test_status, |this| {
                    this.child(
                        div()
                            .h(px(24.))
                            .px_2()
                            .rounded_full()
                            .border_1()
                            .border_color(colors.border_soft)
                            .flex()
                            .items_center()
                            .text_size(px(12.))
                            .text_color(colors.muted)
                            .child(tr("app.connection.status.untested")),
                    )
                }),
        )
        .child(
            Button::new("new-connection-cancel")
                .label(tr("app.common.cancel"))
                .ghost()
                .on_click({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |this, cx| this.cancel_new_connection(cx));
                        cx.stop_propagation();
                    }
                }),
        )
        .child(
            Button::new("new-connection-save-only")
                .debug_selector(|| "new-connection-save-only".into())
                .label(tr("app.connection.save_only"))
                .ghost()
                .disabled(saving || test_busy)
                .on_click({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |this, cx| {
                            this.save_connection_without_test(cx);
                        });
                        cx.stop_propagation();
                    }
                }),
        )
        .child(
            Button::new("new-connection-test")
                .debug_selector(|| "new-connection-test".into())
                .label(tr("app.connection.test"))
                .disabled(test_busy)
                .on_click({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |this, cx| {
                            this.test_new_connection(cx);
                        });
                        cx.stop_propagation();
                    }
                }),
        )
        .child(
            Button::new("new-connection-save")
                .debug_selector(|| "new-connection-save".into())
                .label(tr("app.connection.save_and_connect"))
                .primary()
                .w(px(116.))
                .disabled(saving || test_busy)
                .on_click({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |this, cx| {
                            this.create_connection_from_form(cx);
                        });
                        cx.stop_propagation();
                    }
                }),
        )
}

/// 数据库类型在稳定 `ElementId` 中的后缀（如 `new-connection-kind-postgres`）。
fn database_kind_element_suffix(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::MySql => "mysql",
        DatabaseKind::TiDb => "tidb",
        DatabaseKind::MariaDb => "mariadb",
        DatabaseKind::Sqlite => "sqlite",
        DatabaseKind::MongoDb => "mongodb",
        DatabaseKind::Redis => "redis",
        DatabaseKind::Postgres => "postgres",
        DatabaseKind::Kingbase => "kingbase",
        DatabaseKind::Dameng => "dameng",
        DatabaseKind::SqlServer => "sqlserver",
        DatabaseKind::Oracle => "oracle",
    }
}

fn database_kind_icon_path(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::MySql => "db/mysql.svg",
        DatabaseKind::TiDb => "db/tidb.svg",
        DatabaseKind::MariaDb => "db/mariadb.svg",
        DatabaseKind::Sqlite => "db/sqlite.svg",
        DatabaseKind::MongoDb => "db/mongodb.svg",
        DatabaseKind::Redis => "db/redis.svg",
        DatabaseKind::Postgres => "db/postgres.svg",
        DatabaseKind::Kingbase => "db/kingbase.svg",
        DatabaseKind::Dameng => "db/dm.svg",
        DatabaseKind::SqlServer => "db/sqlserver.svg",
        DatabaseKind::Oracle => "db/oracle.svg",
    }
}

fn new_connection_tab_content(
    tab: NewConnectionTab,
    kind: DatabaseKind,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    match tab {
        NewConnectionTab::Connection => connection_form(kind, form, inputs, colors, window, cx),
        // TLS 与 SSH 合并到「网络与安全」：两者都属于建连链路，单独一页反而来回切。
        NewConnectionTab::Tls => network_security_form(kind, form, inputs, colors, window, cx),
        NewConnectionTab::Advanced => {
            let specific = match kind {
                DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
                    mysql_advanced_form(form, inputs, colors, window, cx)
                }
                DatabaseKind::Postgres | DatabaseKind::Kingbase => {
                    postgres_advanced_form(form, inputs, colors, window, cx)
                }
                DatabaseKind::Redis => advanced_form(form, inputs, colors, window, cx),
                DatabaseKind::SqlServer => sqlserver_advanced_form(inputs, colors, window, cx),
                DatabaseKind::Oracle => oracle_advanced_form(inputs, colors, window, cx),
                // SRV 已移到连接信息；MongoDB 高级页只剩通用只读开关。
                DatabaseKind::MongoDb => div(),
                DatabaseKind::Dameng => dameng_advanced_form(inputs, colors, window, cx),
                DatabaseKind::Sqlite => tab_empty_hint(colors),
            };
            // 只读开关对所有连接类型通用，放在各库专属高级项之前。
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_3()
                .child(form_section(
                    tr("app.connection.section.connection_behavior"),
                    colors,
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            v_form().child(toggle_row_light(
                                tr("app.connection.field.read_only"),
                                ConnectionToggleField::ReadOnly,
                                form.read_only,
                                colors,
                                cx,
                            )),
                        )
                        .child(muted_hint(tr("app.connection.read_only.hint"), colors)),
                ))
                .child(specific)
        }
    }
}

/// 网络与安全页签：TLS/加密 + SSH 隧道按类型拼装，无可用项时显示空态。
fn network_security_form(
    kind: DatabaseKind,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let tls = match kind {
        DatabaseKind::MySql
        | DatabaseKind::TiDb
        | DatabaseKind::MariaDb
        | DatabaseKind::Redis
        | DatabaseKind::Postgres
        | DatabaseKind::Kingbase => Some(tls_form(kind, form, inputs, colors, window, cx)),
        DatabaseKind::SqlServer => Some(sqlserver_encryption_form(inputs, colors, window, cx)),
        // Oracle 的证书校验由 Wallet（含 TCPS 凭据）承担，没有独立开关。
        DatabaseKind::Oracle => Some(oracle_wallet_form(inputs, colors, window, cx)),
        _ => None,
    };
    let ssh = if supports_ssh_tunnel(kind) {
        Some(ssh_form(kind, form, inputs, colors, window, cx))
    } else {
        None
    };
    let mut body = div().w_full().flex().flex_col().gap_3();
    if let Some(tls) = tls {
        body = body.child(tls);
    }
    if let Some(ssh) = ssh {
        body = body.child(ssh);
    }
    if kind == DatabaseKind::Sqlite {
        body = body.child(tab_empty_hint(colors));
    }
    body
}

fn supports_ssh_tunnel(kind: DatabaseKind) -> bool {
    matches!(
        kind,
        DatabaseKind::MySql
            | DatabaseKind::TiDb
            | DatabaseKind::MariaDb
            | DatabaseKind::Redis
            | DatabaseKind::Postgres
            | DatabaseKind::Kingbase
            | DatabaseKind::MongoDb
            | DatabaseKind::SqlServer
            | DatabaseKind::Oracle
            | DatabaseKind::Dameng
    )
}

/// 当前类型没有可配置项时的空态。
fn tab_empty_hint(colors: UiColors) -> Div {
    div()
        .w_full()
        .h(px(96.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .justify_center()
        .px_4()
        .text_size(px(13.))
        .text_color(colors.muted)
        .child(tr("app.connection.no_options"))
}

/// TLS 页签：启用开关 + 证书/私钥文件路径 + SNI + 校验证书。
fn tls_form(
    kind: DatabaseKind,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let enabled = form.tls_enabled;
    // TLS 参数明细只在启用后渲染：仅置灰仍可编辑，但连接器在 TLS 关闭时
    // 一律按 DISABLED / Disable 拨号，用户填的证书与模式会被静默丢弃。
    let body = if enabled {
        tls_parameters_block(kind, form, inputs, colors, window, cx)
    } else {
        muted_hint(tr("app.connection.tls.enable_hint"), colors)
    };
    form_section_with_action(
        tr("app.connection.tls.title"),
        colors,
        Some(toggle_switch(
            ConnectionToggleField::TlsEnabled,
            enabled,
            cx,
        )),
        body,
    )
}

/// TLS 参数明细块：CA 证书 / 客户端证书 / 客户端密钥 / SNI / 校验证书。
/// MySQL/TiDB 额外渲染 ssl_mode 与字符集；PostgreSQL 渲染 PG 的 ssl_mode。
fn tls_parameters_block(
    kind: DatabaseKind,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let is_mysql = matches!(kind, DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb);
    let is_postgres = kind.is_postgres_family();
    // 只有 PostgreSQL（rustls）和 Redis（自建 TLS 流）能指定独立 SNI；
    // MySQL / MongoDB 驱动把 TLS 主机名固定为连接地址。
    let sni_supported = is_postgres || kind == DatabaseKind::Redis;
    // CA 只在真正会被读取的校验方式下显示：
    // MySQL 系仅 verify_ca / verify_identity 下发 ssl-ca，PostgreSQL 仅 verify-ca/verify-full
    // 加载 CA，其余模式（require/prefer 等）即便填了也会被连接器忽略。
    let shows_ca = if is_mysql {
        matches!(
            form.mysql_tls_ssl_mode.trim(),
            "verify_ca" | "verify_identity"
        )
    } else if is_postgres {
        matches!(
            form.pg_tls_ssl_mode.trim().to_ascii_lowercase().as_str(),
            "verify-ca" | "verify-full"
        )
    } else {
        // Redis 启用 TLS 后始终加载 CA。
        true
    };
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .when(is_mysql, |this| {
            this.child(mysql_tls_mode_block(form, inputs, colors, window, cx))
        })
        .when(is_postgres, |this| {
            this.child(postgres_tls_mode_block(inputs, colors, window, cx))
        })
        .when(shows_ca, |this| {
            this.child(file_field_row_light(
                tr("app.connection.tls.ca"),
                ConnectionField::TlsCa,
                tr("app.connection.tls.ca_prompt"),
                inputs,
                colors,
                window,
                cx,
            ))
        })
        .child(two_field_row(
            file_field_row_light(
                tr("app.connection.tls.client_cert"),
                ConnectionField::TlsClientCert,
                tr("app.connection.tls.client_cert_prompt"),
                inputs,
                colors,
                window,
                cx,
            ),
            file_field_row_light(
                tr("app.connection.tls.client_key"),
                ConnectionField::TlsClientKey,
                tr("app.connection.tls.client_key_prompt"),
                inputs,
                colors,
                window,
                cx,
            ),
        ))
        // 加密私钥（PKCS#8）需要口令；明文私钥留空即可。
        .child(field_row_light(
            tr("app.connection.tls.client_key_passphrase"),
            ConnectionField::TlsClientKeyPassphrase,
            inputs,
            true,
            colors,
            window,
            cx,
        ))
        // 驱动不支持独立 SNI（主机名固定取连接地址），不渲染避免误填。
        .when(sni_supported, |this| {
            this.child(field_row_light(
                tr("app.connection.tls.sni"),
                ConnectionField::TlsSni,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
        })
        // PostgreSQL 与 MySQL 的证书校验由 ssl_mode 决定（verify-ca/verify-*），
        // 该开关只对 Redis 生效，PG/MySQL 上不渲染，避免模式与开关两套事实来源。
        .when(!is_postgres && !is_mysql, |this| {
            this.child(
                v_form().child(toggle_row_light(
                    tr("app.connection.tls.verify"),
                    ConnectionToggleField::TlsVerify,
                    form.tls_verify,
                    colors,
                    cx,
                )),
            )
        })
        .when(!shows_ca, |this| {
            this.child(muted_hint(tr("app.connection.tls.ca_hint"), colors))
        })
}

/// MySQL/TiDB TLS 模式块：只放 SSL 模式；字符集属于低频连接参数，
/// 统一放在高级页「连接参数」，避免同一字段在两个页签各渲染一次。
fn mysql_tls_mode_block(
    _form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    _colors: UiColors,
    _window: &mut Window,
    _cx: &mut Context<NavicatMain>,
) -> Div {
    div().w_full().child(
        v_form().child(connection_select_field(
            tr("app.connection.tls.ssl_mode"),
            &inputs.mysql_ssl_mode_select,
            "",
        )),
    )
}

/// PostgreSQL 的 TLS 模式（disable/prefer/require/verify-ca/verify-full）。
fn postgres_tls_mode_block(
    inputs: &NewConnectionInputs,
    _colors: UiColors,
    _window: &mut Window,
    _cx: &mut Context<NavicatMain>,
) -> Div {
    div().w_full().child(
        v_form()
            
            .child(connection_select_field(
                tr("app.connection.tls.ssl_mode"),
                &inputs.pg_ssl_mode_select,
                "",
            )),
    )
}

/// SQL Server TLS 页签：加密模式；默认信任服务器证书，不额外提供 CA 路径。
fn sqlserver_encryption_form(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    _window: &mut Window,
    _cx: &mut Context<NavicatMain>,
) -> Div {
    form_section(
        tr("app.connection.sqlserver.encryption"),
        colors,
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                v_form().child(connection_select_field(
                    tr("app.connection.sqlserver.encryption"),
                    &inputs.sqlserver_encryption_select,
                    "",
                )),
            )
            .child(muted_hint(
                tr("app.connection.sqlserver.encryption_hint"),
                colors,
            )),
    )
}

/// SQL Server 高级页签：超时与应用名。
fn sqlserver_advanced_form(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .child(form_section(
            tr("app.connection.section.timeouts"),
            colors,
            two_field_row(
                field_row_light(
                    tr("app.connection.sqlserver.connect_timeout"),
                    ConnectionField::SqlServerConnectTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ),
                field_row_light(
                    tr("app.connection.sqlserver.query_timeout"),
                    ConnectionField::SqlServerQueryTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ),
            ),
        ))
        .child(form_section(
            tr("app.connection.section.session"),
            colors,
            v_form().child(field_row_light(
                tr("app.connection.sqlserver.application_name"),
                ConnectionField::SqlServerApplicationName,
                inputs,
                false,
                colors,
                window,
                cx,
            )),
        ))
}

/// Oracle TLS 页签：Wallet 目录与口令。
/// Oracle 的证书校验通过 Wallet 承载，这里不提供「跳过校验」开关。
fn oracle_wallet_form(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    form_section(
        tr("app.connection.oracle.wallet_title"),
        colors,
        v_form()
            .child(field_row_light(
                tr("app.connection.oracle.wallet_location"),
                ConnectionField::OracleWalletLocation,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                tr("app.connection.oracle.wallet_password"),
                ConnectionField::OracleWalletPassword,
                inputs,
                true,
                colors,
                window,
                cx,
            )),
    )
}

/// Oracle 高级页签：建连超时、查询超时与应用名。
fn oracle_advanced_form(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .child(form_section(
            tr("app.connection.section.timeouts"),
            colors,
            two_field_row(
                field_row_light(
                    tr("app.connection.oracle.connect_timeout"),
                    ConnectionField::OracleConnectTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ),
                field_row_light(
                    tr("app.connection.oracle.query_timeout"),
                    ConnectionField::OracleQueryTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ),
            ),
        ))
        .child(form_section(
            tr("app.connection.section.session"),
            colors,
            v_form().child(field_row_light(
                tr("app.connection.oracle.application_name"),
                ConnectionField::OracleApplicationName,
                inputs,
                false,
                colors,
                window,
                cx,
            )),
        ))
}

/// PostgreSQL 高级页签：会话、建连/查询超时、代理。
fn postgres_advanced_form(
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let proxy_enabled = form.mysql_proxy_enabled;
    // 代理参数与 MySQL 共用一套编辑字段/档案构建分支，未启用时不渲染明细。
    let proxy_body = if proxy_enabled {
        mysql_proxy_block(form, inputs, colors, window, cx)
    } else {
        div().w_full().child(muted_hint(
            tr("app.connection.mysql.proxy_enable_hint"),
            colors,
        ))
    };
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .child(form_section(
            tr("app.connection.section.session"),
            colors,
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_3()
                .child(two_field_row(
                    field_row_light(
                        tr("app.connection.postgres.application_name"),
                        ConnectionField::PgApplicationName,
                        inputs,
                        false,
                        colors,
                        window,
                        cx,
                    ),
                    field_row_light(
                        tr("app.connection.postgres.timezone"),
                        ConnectionField::PgTimezone,
                        inputs,
                        false,
                        colors,
                        window,
                        cx,
                    ),
                ))
                .child(
                    v_form()
                        .child(toggle_row_light(
                            tr("app.connection.postgres.show_other_databases"),
                            ConnectionToggleField::PgShowOtherDatabases,
                            form.pg_show_other_databases,
                            colors,
                            cx,
                        ))
                        .child(toggle_row_light(
                            tr("app.connection.postgres.show_system_schemas"),
                            ConnectionToggleField::PgShowSystemSchemas,
                            form.pg_show_system_schemas,
                            colors,
                            cx,
                        )),
                ),
        ))
        .child(form_section(
            tr("app.connection.section.timeouts"),
            colors,
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_3()
                .child(two_field_row(
                    field_row_light(
                        tr("app.connection.postgres.connect_timeout"),
                        ConnectionField::PgConnectTimeoutSecs,
                        inputs,
                        false,
                        colors,
                        window,
                        cx,
                    ),
                    field_row_light(
                        tr("app.connection.postgres.query_timeout"),
                        ConnectionField::PgQueryTimeoutSecs,
                        inputs,
                        false,
                        colors,
                        window,
                        cx,
                    ),
                ))
                .child(
                    v_form().child(toggle_row_light(
                        tr("app.connection.postgres.tcp_keepalive"),
                        ConnectionToggleField::PgTcpKeepalive,
                        form.pg_tcp_keepalive,
                        colors,
                        cx,
                    )),
                ),
        ))
        .child(form_section_with_action(
            tr("app.connection.mysql.proxy"),
            colors,
            Some(toggle_switch(
                ConnectionToggleField::MysqlProxyEnabled,
                proxy_enabled,
                cx,
            )),
            proxy_body,
        ))
}

/// SSH 页签：启用开关 + 跳板机参数 + 认证方式。
fn ssh_form(
    kind: DatabaseKind,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 认证方式："password" 显示/可用密码，私钥方式显示/可用私钥与口令。
    let password_mode = form.ssh_auth != "private_key";
    let enabled = form.ssh_enabled;
    let shows_tuning = matches!(
        kind,
        DatabaseKind::MySql
            | DatabaseKind::TiDb
            | DatabaseKind::MariaDb
            | DatabaseKind::MongoDb
            | DatabaseKind::SqlServer
            | DatabaseKind::Oracle
            | DatabaseKind::Dameng
            | DatabaseKind::Postgres
            | DatabaseKind::Kingbase
    );
    // 隧道参数只在启用后渲染：置灰仍可编辑，但未启用时整套 SSH 档案
    // 都不会参与拨号，填了也只会被静默保存。
    let body = if enabled {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(ssh_tunnel_block(form, inputs, colors, window, cx))
            // SSH 连接超时 + 心跳间隔（后端结构化档案已支持的类型）。
            .when(shows_tuning, |this| {
                this.child(mysql_ssh_tuning_block(inputs, colors, window, cx))
            })
            // 只渲染当前认证方式对应的区块，另一种方式不出现。
            .when(password_mode, |this| {
                this.child(ssh_password_block(inputs, colors, window, cx))
            })
            .when(!password_mode, |this| {
                this.child(ssh_private_key_block(inputs, colors, window, cx))
            })
    } else {
        div().w_full().child(muted_hint(
            tr("app.connection.ssh.enable_hint"),
            colors,
        ))
    };
    form_section_with_action(
        tr("app.connection.ssh.title"),
        colors,
        Some(toggle_switch(
            ConnectionToggleField::SshEnabled,
            enabled,
            cx,
        )),
        body,
    )
}

/// SSH 隧道参数块：主机 / 端口 / 用户名 / 认证方式。
fn ssh_tunnel_block(
    _form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 与「连接信息」页一致：标签在上、输入框在下，主机/端口与用户名/认证方式各成一行。
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .child(two_field_row(
            field_row_light(
                tr("app.connection.field.host"),
                ConnectionField::SshHost,
                inputs,
                false,
                colors,
                window,
                cx,
            ),
            field_row_light(
                tr("app.connection.field.port"),
                ConnectionField::SshPort,
                inputs,
                false,
                colors,
                window,
                cx,
            ),
        ))
        .child(two_field_row(
            field_row_light(
                tr("app.connection.field.username"),
                ConnectionField::SshUsername,
                inputs,
                false,
                colors,
                window,
                cx,
            ),
            connection_select_field(
                tr("app.connection.ssh.auth_method"),
                &inputs.ssh_auth_select,
                "",
            ),
        ))
        // 主机密钥策略：默认严格校验 known_hosts；首次连接可显式接受并写入。
        .child(connection_select_field(
            tr("app.connection.ssh.host_key_policy"),
            &inputs.ssh_host_key_policy_select,
            "",
        ))
}

/// SSH 密码认证块。
fn ssh_password_block(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w_full()
        .child(v_form().child(field_row_light(
            tr("app.connection.field.password"),
            ConnectionField::SshPassword,
            inputs,
            true,
            colors,
            window,
            cx,
        )))
}

/// SSH 私钥认证块：私钥文件 + 口令。
fn ssh_private_key_block(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div().w_full().child(
        v_form()
            
            .child(file_field_row_light(
                tr("app.connection.ssh.private_key"),
                ConnectionField::SshPrivateKey,
                tr("app.connection.ssh.private_key_prompt"),
                inputs,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                tr("app.connection.ssh.passphrase"),
                ConnectionField::SshPassphrase,
                inputs,
                true,
                colors,
                window,
                cx,
            )),
    )
}

/// MySQL/TiDB SSH 调优块：连接超时 + 心跳间隔。
fn mysql_ssh_tuning_block(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .child(redis_section_label(tr("app.connection.ssh.tuning"), colors))
        .child(two_field_row(
            field_row_light(
                tr("app.connection.ssh.connect_timeout"),
                ConnectionField::MysqlSshConnectTimeout,
                inputs,
                false,
                colors,
                window,
                cx,
            ),
            field_row_light(
                tr("app.connection.ssh.keepalive"),
                ConnectionField::MysqlSshKeepalive,
                inputs,
                false,
                colors,
                window,
                cx,
            ),
        ))
}

/// 达梦高级页：建连超时、查询超时与应用名。
///
/// `encoding` 不暴露：手工填错会导致乱码，档案值在编辑保存时原样保留。
fn dameng_advanced_form(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .child(form_section(
            tr("app.connection.section.timeouts"),
            colors,
            two_field_row(
                field_row_light(
                    tr("app.connection.dameng.connect_timeout"),
                    ConnectionField::DamengConnectTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ),
                field_row_light(
                    tr("app.connection.dameng.query_timeout"),
                    ConnectionField::DamengQueryTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ),
            ),
        ))
        .child(form_section(
            tr("app.connection.section.session"),
            colors,
            v_form().child(field_row_light(
                tr("app.connection.dameng.application_name"),
                ConnectionField::DamengApplicationName,
                inputs,
                false,
                colors,
                window,
                cx,
            )),
        ))
}

/// MySQL/TiDB 高级页：连接参数、会话、超时与回收、代理。
fn mysql_advanced_form(
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let proxy_enabled = form.mysql_proxy_enabled;
    // 代理参数只在启用后渲染：未启用时代理档案不会参与拨号。
    let proxy_body = if proxy_enabled {
        mysql_proxy_block(form, inputs, colors, window, cx)
    } else {
        div().w_full().child(muted_hint(
            tr("app.connection.mysql.proxy_enable_hint"),
            colors,
        ))
    };
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        // 字符集、排序规则与 URL 参数只在特殊环境才需要改，移出连接信息首屏。
        .child(form_section(
            tr("app.connection.section.connection_parameters"),
            colors,
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_3()
                .child(field_row_light(
                    tr("app.connection.field.url_params"),
                    ConnectionField::UrlParams,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(two_field_row(
                    field_row_light(
                        tr("app.connection.tls.charset"),
                        ConnectionField::MysqlCharset,
                        inputs,
                        false,
                        colors,
                        window,
                        cx,
                    ),
                    field_row_light(
                        tr("app.connection.mysql.collation"),
                        ConnectionField::MysqlCollation,
                        inputs,
                        false,
                        colors,
                        window,
                        cx,
                    ),
                )),
        ))
        // 会话初始化：建连后立即生效，直接影响用户 SQL 的时区解释。
        .child(form_section(
            tr("app.connection.section.session"),
            colors,
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_3()
                .child(field_row_light(
                    tr("app.connection.mysql.timezone"),
                    ConnectionField::MysqlTimezone,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.mysql.init_sql"),
                    ConnectionField::MysqlInitSql,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        ))
        .child(form_section(
            tr("app.connection.section.timeouts"),
            colors,
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_3()
                .child(two_field_row(
                    field_row_light(
                        tr("app.connection.mysql.connect_timeout"),
                        ConnectionField::MysqlConnectTimeout,
                        inputs,
                        false,
                        colors,
                        window,
                        cx,
                    ),
                    field_row_light(
                        tr("app.connection.mysql.query_timeout"),
                        ConnectionField::MysqlQueryTimeout,
                        inputs,
                        false,
                        colors,
                        window,
                        cx,
                    ),
                ))
                .child(
                    v_form().child(toggle_row_light(
                        tr("app.connection.mysql.tcp_keepalive"),
                        ConnectionToggleField::MysqlTcpKeepalive,
                        form.mysql_tcp_keepalive,
                        colors,
                        cx,
                    )),
                ),
        ))
        .child(form_section_with_action(
            tr("app.connection.mysql.proxy"),
            colors,
            Some(toggle_switch(
                ConnectionToggleField::MysqlProxyEnabled,
                proxy_enabled,
                cx,
            )),
            proxy_body,
        ))
}

/// MySQL/TiDB 代理参数块：类型 / 主机 / 端口 / 用户名 / 密码。
fn mysql_proxy_block(
    _form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 与「连接信息」页一致：标签在上、输入框在下；类型、地址、认证各成一行。
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            v_form().child(connection_select_field(
                tr("app.connection.mysql.proxy_type"),
                &inputs.mysql_proxy_type_select,
                "",
            )),
        )
        .child(two_field_row(
            field_row_light(
                tr("app.connection.field.host"),
                ConnectionField::MysqlProxyHost,
                inputs,
                false,
                colors,
                window,
                cx,
            ),
            field_row_light(
                tr("app.connection.field.port"),
                ConnectionField::MysqlProxyPort,
                inputs,
                false,
                colors,
                window,
                cx,
            ),
        ))
        .child(two_field_row(
            field_row_light(
                tr("app.connection.mysql.proxy_username"),
                ConnectionField::MysqlProxyUsername,
                inputs,
                false,
                colors,
                window,
                cx,
            ),
            field_row_light(
                tr("app.connection.mysql.proxy_password"),
                ConnectionField::MysqlProxyPassword,
                inputs,
                true,
                colors,
                window,
                cx,
            ),
        ))
}

/// 高级页签：拓扑覆盖项（Sentinel 主库名 / 强制单机 / 集群只读）与 AI Key 范围。
/// 连接串导入只在「连接信息」页出现一次，避免同一输入框两处渲染。
fn advanced_form(
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 拓扑由连接器自动探测，这里只保留探测不到时需要的少量覆盖项：
    // 主库名（多主库哨兵）、强制单机（代理误判逃生口）、集群只读。
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .child(form_section(
            tr("app.connection.redis.connect_behavior"),
            colors,
            v_form()
                .child(
                    field_row_light(
                        tr("app.connection.redis.sentinel_master_optional"),
                        ConnectionField::SentinelMasterName,
                        inputs,
                        false,
                        colors,
                        window,
                        cx,
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.muted)
                            .child(tr("app.connection.redis.sentinel_master_hint")),
                    ),
                )
                .child(
                    toggle_row_light(
                        tr("app.connection.redis.force_standalone"),
                        ConnectionToggleField::ForceStandalone,
                        form.force_standalone,
                        colors,
                        cx,
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.muted)
                            .child(tr("app.connection.redis.force_standalone_help")),
                    ),
                )
                .child(toggle_row_light(
                    tr("app.connection.redis.cluster_readonly"),
                    ConnectionToggleField::ClusterAllowReadonly,
                    form.cluster_allow_readonly,
                    colors,
                    cx,
                )),
        ))
        .child(form_section(
            tr("app.connection.redis.ai_scope"),
            colors,
            v_form().child(
                field_row_light(
                    tr("app.connection.redis.ai_key_prefixes"),
                    ConnectionField::AiKeyPrefixes,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(tr("app.connection.redis.ai_key_prefixes_help")),
                ),
            ),
        ))
}

/// 连接串导入框：虚线容器 + 链接图标 + 无边框输入 + 「解析填入」按钮。
fn connection_uri_box(
    kind: DatabaseKind,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let action_label = match kind {
        DatabaseKind::MongoDb => tr("app.connection.mongo.import_action"),
        DatabaseKind::Redis => tr("app.connection.redis.import_action"),
        _ => tr("app.connection.uri.parse_action"),
    };
    let accent = if colors.is_dark {
        rgb(0x6b9bff)
    } else {
        rgb(0x3b76f6)
    };
    let accent_soft = if colors.is_dark {
        rgb(0x1d2b47)
    } else {
        rgb(0xeaf1ff)
    };
    div()
        .w_full()
        .rounded(colors.radius_lg + px(4.))
        .border_1()
        .border_dashed()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .px_3()
        .py_2()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .size(px(28.))
                .flex_none()
                .rounded(colors.radius_lg)
                .bg(accent_soft)
                .flex()
                .items_center()
                .justify_center()
                .child(app_icon_box(AppIcon::Link, 28., 15., accent)),
        )
        // 草案里连接串是虚线容器内的独立输入框，而不是贴着容器底色的裸输入。
        .child(
            input_box_light(
                ConnectionField::DiscoveryUri,
                inputs,
                false,
                colors,
                window,
                cx,
            )
            .flex_1()
            .min_w(px(0.)),
        )
        .child(
            div()
                .debug_selector(|| "connection-uri-import".into())
                .h(px(36.))
                .flex_none()
                .px_4()
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .flex()
                .items_center()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .child(action_label)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        // 当前选中的数据库类型决定走哪套 URI 解析。
                        match this.new_connection_kind {
                            Some(DatabaseKind::MongoDb) => this.import_mongo_connection_string(cx),
                            Some(DatabaseKind::Redis) => this.import_redis_connection_string(cx),
                            // PostgreSQL 有独立的 URI 解析：sslmode/证书/应用名/超时等
                            // 都能落到档案；通用 SQL 解析会丢弃这些 query 参数。
                            Some(DatabaseKind::Postgres | DatabaseKind::Kingbase) => {
                                this.import_postgres_connection_string(cx)
                            }
                            _ => this.import_sql_connection_string(cx),
                        }
                        cx.stop_propagation();
                    }),
                ),
        )
}

/// 小节标题。
fn redis_section_label(text: impl Into<SharedString>, colors: UiColors) -> Div {
    let text = text.into();
    div()
        .pt_1()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(text)
}

/// 控件未启用时的一行灰色说明，替代「置灰但可编辑」的假禁用态。
fn muted_hint(text: impl Into<SharedString>, colors: UiColors) -> Div {
    div()
        .text_size(px(12.))
        .text_color(colors.muted)
        .child(text.into())
}

/// 新建连接弹框里的下拉行：用 gpui-component Select 渲染，绑定根实体上的 SelectState 光标。
fn connection_select_field<I: SelectItem + 'static>(
    label: impl Into<SharedString>,
    select: &Entity<SelectState<SearchableVec<I>>>,
    search_placeholder: impl Into<SharedString>,
) -> Field {
    field()
        .label(label.into())
        .child(
        Select::new(select)
            .small()
            .search_placeholder(search_placeholder),
    )
}

/// 布尔开关（Switch 样式）：点击写回表单对应开关字段。
/// 外层 Div 承载 E2E 的 debug selector，保证点击落点仍只覆盖开关本身。
fn toggle_switch(
    field_id: ConnectionToggleField,
    checked: bool,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let view = cx.entity();
    let id = connection_toggle_id(field_id);
    let debug_id = id.to_string();
    div()
        .flex_none()
        .debug_selector(move || debug_id.clone())
        .child(
            Switch::new(id)
                .checked(checked)
                .on_click(move |new_checked, window, cx| {
                    let new_checked = *new_checked;
                    let _ = view.update(cx, |this, cx| {
                        this.set_connection_toggle_field(field_id, new_checked, window, cx);
                    });
                }),
        )
}

/// 布尔开关行：开关在左、文案在右，与页签卡片头的开关样式一致。
fn toggle_row_light(
    label: impl Into<SharedString>,
    field_id: ConnectionToggleField,
    checked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Field {
    field().child(
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .child(toggle_switch(field_id, checked, cx))
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(colors.text)
                    .child(label.into()),
            ),
    )
}

/// 文件选择行：文件路径输入 + 「选择文件」按钮。
fn file_field_row_light(
    label: impl Into<SharedString>,
    field_id: ConnectionField,
    prompt: impl Into<SharedString>,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Field {
    let prompt = prompt.into().to_string();
    field().label(label.into()).child(
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .child(
                input_box_light(field_id, inputs, false, colors, window, cx)
                    .flex_1()
                    .min_w(px(0.)),
            )
            .child(file_picker_button_light(field_id, prompt, colors, cx).flex_none()),
    )
}

/// 「选择文件」按钮：调用对应字段的通用文件选择器。
fn file_picker_button_light(
    field_id: ConnectionField,
    prompt: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .size(px(38.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover).border_color(rgb(0xaeb7c2)))
        .child(
            div()
                .relative()
                .w(px(18.))
                .h(px(14.))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(2.))
                        .w(px(8.))
                        .h(px(4.))
                        .rounded(colors.radius * 0.5)
                        .bg(colors.muted),
                )
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w(px(18.))
                        .h(px(12.))
                        .rounded(colors.radius * 0.5)
                        .border_2()
                        .border_color(colors.muted)
                        .bg(colors.input_bg),
                ),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.choose_connection_file(field_id, prompt.clone(), window, cx);
                cx.stop_propagation();
            }),
        )
}

/// 布尔开关的稳定元素 ID。
fn connection_toggle_id(field: ConnectionToggleField) -> &'static str {
    match field {
        ConnectionToggleField::TlsEnabled => "new-connection-toggle-tls",
        ConnectionToggleField::TlsVerify => "new-connection-toggle-tls-verify",
        ConnectionToggleField::SshEnabled => "new-connection-toggle-ssh",
        ConnectionToggleField::ClusterAllowReadonly => "new-connection-toggle-cluster-readonly",
        ConnectionToggleField::ForceStandalone => "new-connection-toggle-force-standalone",
        ConnectionToggleField::MysqlProxyEnabled => "new-connection-toggle-mysql-proxy",
        ConnectionToggleField::MysqlTcpKeepalive => "new-connection-toggle-mysql-tcp-keepalive",
        ConnectionToggleField::PgTcpKeepalive => "new-connection-toggle-pg-tcp-keepalive",
        ConnectionToggleField::PgShowOtherDatabases => {
            "new-connection-toggle-pg-show-other-databases"
        }
        ConnectionToggleField::PgShowSystemSchemas => {
            "new-connection-toggle-pg-show-system-schemas"
        }
        ConnectionToggleField::MongoSrv => "new-connection-toggle-mongo-srv",
        ConnectionToggleField::ReadOnly => "new-connection-toggle-read-only",
    }
}

/// 连接信息页：按类型拼装两列表单，主机与端口同格，其余字段按语义成对。
fn connection_form(
    kind: DatabaseKind,
    form_state: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut body = div().w_full().flex().flex_col().gap_3();
    // 基础信息统一放在一张「服务器」卡片里：两列栅格（列距 14、行距 10），与设计稿一致。
    let mut fields = if kind == DatabaseKind::Sqlite {
        v_form().gap_y(px(10.))
    } else {
        v_form().columns(2).gap_x(px(14.)).gap_y(px(10.))
    };
    // 少数类型在字段下方补一行说明（如 Redis 拓扑自动识别）。
    let mut footer_hint: Option<SharedString> = None;

    // 连接串解析不再是 Mongo/Redis 专属：SQL 类型同样按 scheme 回填，省掉逐项手填。
    if kind != DatabaseKind::Sqlite {
        body = body.child(connection_uri_box(kind, inputs, colors, window, cx));
    }

    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
            fields = fields
                .child(host_port_row_light(
                    tr("app.connection.field.host_port"),
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.database"),
                    ConnectionField::Database,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.username"),
                    ConnectionField::Username,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.password"),
                    ConnectionField::Password,
                    inputs,
                    true,
                    colors,
                    window,
                    cx,
                ))
                // Unix socket 与 TCP 主机/端口二选一：填写后忽略主机/端口拨号。
                .child(field_row_light(
                    tr("app.connection.mysql.socket_path"),
                    ConnectionField::MysqlSocketPath,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ));
        }
        DatabaseKind::Postgres | DatabaseKind::Kingbase => {
            fields = fields
                .child(host_port_row_light(
                    tr("app.connection.field.host_port"),
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.database"),
                    ConnectionField::Database,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.username"),
                    ConnectionField::Username,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.password"),
                    ConnectionField::Password,
                    inputs,
                    true,
                    colors,
                    window,
                    cx,
                ))
                // 默认 schema 与数据库同属连接后的默认作用域，放在首屏。
                .child(field_row_light(
                    tr("app.connection.postgres.default_schema"),
                    ConnectionField::PgDefaultSchema,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ));
        }
        DatabaseKind::Dameng => {
            fields = fields
                .child(host_port_row_light(
                    tr("app.connection.field.host_port"),
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.dameng.schema"),
                    ConnectionField::DamengSchema,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.username"),
                    ConnectionField::Username,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.password"),
                    ConnectionField::Password,
                    inputs,
                    true,
                    colors,
                    window,
                    cx,
                ));
        }
        // Oracle：Service Name 与 SID 互斥，只渲染当前寻址方式对应的输入框。
        DatabaseKind::Oracle => {
            let target = if form_state.oracle_connect_kind == "sid" {
                field_row_light(
                    tr("app.connection.oracle.sid"),
                    ConnectionField::OracleSid,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )
            } else {
                field_row_light(
                    tr("app.connection.oracle.service_name"),
                    ConnectionField::OracleServiceName,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )
            };
            fields = fields
                // 连接方式决定下一个目标字段是 Service Name 还是 SID，必须排在它前面。
                .child(connection_select_field(
                    tr("app.connection.oracle.connect_kind"),
                    &inputs.oracle_connect_kind_select,
                    "",
                ))
                .child(host_port_row_light(
                    tr("app.connection.field.host_port"),
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(target)
                .child(field_row_light(
                    tr("app.connection.field.username"),
                    ConnectionField::Username,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.password"),
                    ConnectionField::Password,
                    inputs,
                    true,
                    colors,
                    window,
                    cx,
                ))
                .child(connection_select_field(
                    tr("app.connection.oracle.privilege"),
                    &inputs.oracle_privilege_select,
                    "",
                ))
                .child(field_row_light(
                    tr("app.connection.oracle.schema"),
                    ConnectionField::OracleSchema,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ));
        }
        DatabaseKind::SqlServer => {
            fields = fields
                .child(host_port_row_light(
                    tr("app.connection.field.host_port"),
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.sqlserver.instance"),
                    ConnectionField::SqlServerInstance,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.database"),
                    ConnectionField::Database,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.sqlserver.schema"),
                    ConnectionField::SqlServerSchema,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.username"),
                    ConnectionField::Username,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.password"),
                    ConnectionField::Password,
                    inputs,
                    true,
                    colors,
                    window,
                    cx,
                ));
        }
        DatabaseKind::MongoDb => {
            fields = fields
                // SRV 改变地址语义（单 DNS 名 + 拓扑发现），必须和主机/端口放在一起。
                .child(
                    toggle_row_light(
                        tr("app.connection.mongo.srv"),
                        ConnectionToggleField::MongoSrv,
                        form_state.mongo_srv,
                        colors,
                        cx,
                    )
                    .col_span(2),
                )
                .child(host_port_row_light(
                    tr("app.connection.field.host_port"),
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.default_db"),
                    ConnectionField::MongoDefaultDb,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.username"),
                    ConnectionField::Username,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.password"),
                    ConnectionField::Password,
                    inputs,
                    true,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.auth_db"),
                    ConnectionField::MongoAuthDb,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(connection_select_field(
                    tr("app.connection.field.auth_mechanism"),
                    &inputs.mongo_auth_mechanism_select,
                    "",
                ));
        }
        DatabaseKind::Redis => {
            // 只有一个地址入口：填单机节点、哨兵地址或集群任一节点都行，
            // 连接时自动识别拓扑，不再让用户在另一个页签重复选模式。
            fields = fields
                .child(host_port_row_light(
                    tr("app.connection.field.host_port"),
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.username"),
                    ConnectionField::Username,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.field.password"),
                    ConnectionField::Password,
                    inputs,
                    true,
                    colors,
                    window,
                    cx,
                ));
            footer_hint = Some(tr("app.connection.redis.auto_detect_hint"));
        }
        DatabaseKind::Sqlite => {
            fields = fields.child(sqlite_file_row_light(inputs, colors, window, cx));
        }
    }

    let mut section = div().w_full().flex().flex_col().gap_3().child(fields);
    if let Some(hint) = footer_hint {
        section = section.child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(hint),
        );
    }
    // SQLite 的字段是「数据库文件」，其余类型统一叫「服务器」。
    let section_title = if kind == DatabaseKind::Sqlite {
        tr("app.connection.group.file")
    } else {
        tr("app.connection.group.server")
    };
    body.child(form_section(section_title, colors, section))
}

/// 小节容器：浅色底 + 「标题 ————」分隔线，内部直接放表单网格。
fn form_section(title: SharedString, colors: UiColors, content: impl IntoElement) -> Div {
    form_section_with_action(title, colors, None, content)
}

/// 带头部操作的小节容器：操作（通常是启用开关）放在分隔线右侧。
fn form_section_with_action(
    title: SharedString,
    colors: UiColors,
    action: Option<Div>,
    content: impl IntoElement,
) -> Div {
    div()
        .w_full()
        .rounded(colors.radius_lg + px(4.))
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_alt)
        .px_4()
        .pt_3()
        .pb_3()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(colors.muted)
                        .child(title),
                )
                .child(div().flex_1().h(px(1.)).bg(colors.border_soft))
                .when_some(action, |this, action| this.child(action)),
        )
        .child(content)
}

/// 两列表单行：同一行放两个等宽字段，窄窗口下各自收缩不撑破卡片。
fn two_field_row(left: impl IntoElement, right: impl IntoElement) -> Div {
    div()
        .w_full()
        .flex()
        .gap_3()
        .child(div().flex_1().min_w(px(0.)).child(left))
        .child(div().flex_1().min_w(px(0.)).child(right))
}
