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
                .w(px(820.))
                .h(px(560.))
                .rounded(colors.radius_lg)
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
                .child(
                    div().absolute().top(px(12.)).right(px(14.)).child(
                        Button::new("new-connection-close")
                            .label("×")
                            .ghost()
                            .w(px(34.))
                            .h(px(34.))
                            .text_size(px(24.))
                            .on_click({
                                let view = view.clone();
                                move |_, _, cx| {
                                    view.update(cx, |this, cx| {
                                        this.cancel_new_connection(cx);
                                    });
                                    cx.stop_propagation();
                                }
                            }),
                    ),
                )
                .child(
                    div()
                        .h(px(62.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(20.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(div().w(px(34.))),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .gap_5()
                        .px_5()
                        .pb_2()
                        .overflow_hidden()
                        .child(
                            div()
                                .w(px(350.))
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .child(
                                    div()
                                        .h(px(34.))
                                        .rounded(colors.radius_lg)
                                        .border_1()
                                        .border_color(colors.border)
                                        .bg(colors.input_bg)
                                        .px_3()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .text_size(px(14.))
                                        .text_color(colors.muted)
                                        .child(app_icon(AppIcon::Search, 14., colors.muted))
                                        .child(tr("app.connection.search_kind")),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap_3()
                                        .child(kind_tile(
                                            "MySQL",
                                            DatabaseKind::MySql,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        ))
                                        .child(kind_tile(
                                            "MariaDB",
                                            DatabaseKind::MariaDb,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        ))
                                        .child(kind_tile(
                                            "TiDB",
                                            DatabaseKind::TiDb,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        ))
                                        .child(kind_tile(
                                            "SQLite",
                                            DatabaseKind::Sqlite,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        )),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap_3()
                                        .child(kind_tile(
                                            "PostgreSQL",
                                            DatabaseKind::Postgres,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        ))
                                        .child(kind_tile(
                                            "DM8",
                                            DatabaseKind::Dameng,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        ))
                                        .child(kind_tile(
                                            "SQL Server",
                                            DatabaseKind::SqlServer,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        )),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap_3()
                                        .child(kind_tile(
                                            "Redis",
                                            DatabaseKind::Redis,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        ))
                                        .child(kind_tile(
                                            "MongoDB",
                                            DatabaseKind::MongoDb,
                                            kind,
                                            false,
                                            colors,
                                            cx,
                                        ))
                                        .child(kind_tile(
                                            "Oracle",
                                            DatabaseKind::Oracle,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .overflow_hidden()
                                .child(connection_tab_bar(active_tab, colors, cx))
                                .child(div().h(px(1.)).bg(colors.border))
                                .child(
                                    div()
                                        .id("new-connection-form-scroll")
                                        .flex_1()
                                        .min_h(px(0.))
                                        .overflow_scroll()
                                        .scrollbar_width(px(8.))
                                        .child(new_connection_tab_content(
                                            active_tab, kind, form, inputs, colors, window, cx,
                                        )),
                                ),
                        ),
                )
                .child(
                    div()
                        .h(px(58.))
                        .flex_none()
                        .flex()
                        .items_end()
                        .gap_3()
                        .px_5()
                        .pb_5()
                        .when_some(form.test_status.as_ref(), |this, status| {
                            this.child(
                                connection_test_status(status)
                                    .flex_1()
                                    .min_w(px(0.))
                                    .justify_start(),
                            )
                        })
                        .when_none(&form.test_status, |this| {
                            this.child(div().flex_1().min_w(px(0.)))
                        })
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
                        ),
                ),
        )
}

fn kind_tile(
    label: &'static str,
    kind: DatabaseKind,
    selected: DatabaseKind,
    locked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let selected = kind == selected;
    let disabled = locked && !selected;

    // ID 按数据库类型命名而非文案，便于 UI 自动化稳定定位「切换到哪种数据库」。
    // 磁贴可点击切换，纳入 headless 观察。
    div()
        .id(format!(
            "new-connection-kind-{}",
            database_kind_element_suffix(kind)
        ))
        .debug_selector(move || {
            format!("new-connection-kind-{}", database_kind_element_suffix(kind))
        })
        .aria_label(label)
        .role(Role::RadioButton)
        .aria_selected(selected)
        .w(px(168.))
        .h(px(112.))
        .rounded(colors.radius_lg)
        .bg(if disabled {
            if colors.is_dark {
                rgb(0x1b1e23)
            } else {
                rgb(0xf1f2f4)
            }
        } else if selected {
            if colors.is_dark {
                rgb(0x1a3157)
            } else {
                rgb(0xe9f1ff)
            }
        } else {
            colors.panel_alt
        })
        .border_1()
        .border_color(if disabled {
            if colors.is_dark {
                rgb(0x2b3038)
            } else {
                rgb(0xd6dae0)
            }
        } else if selected {
            rgb(0x5b8def)
        } else {
            colors.border
        })
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .when(!locked, |this| {
            this.hover(move |style| {
                style
                    .bg(if selected {
                        if colors.is_dark {
                            rgb(0x213c66)
                        } else {
                            rgb(0xe3edff)
                        }
                    } else {
                        colors.hover
                    })
                    .border_color(if selected {
                        rgb(0x3478f6)
                    } else {
                        rgb(0xb9c0ca)
                    })
            })
        })
        .child(
            div()
                .size(px(48.))
                .rounded(colors.radius_lg)
                .bg(if disabled {
                    if colors.is_dark {
                        rgb(0x24272c)
                    } else {
                        rgb(0xe3e5e8)
                    }
                } else if selected {
                    rgb(0x24272d)
                } else {
                    rgb(0x2a2d33)
                })
                .flex()
                .items_center()
                .justify_center()
                .child(database_kind_icon(kind)),
        )
        .child(
            div()
                .text_size(px(16.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if disabled {
                    if colors.is_dark {
                        rgb(0x6f7783)
                    } else {
                        rgb(0x9aa1ac)
                    }
                } else if selected {
                    if colors.is_dark {
                        rgb(0x9cc2ff)
                    } else {
                        rgb(0x1d4f91)
                    }
                } else {
                    colors.text
                })
                .child(label),
        )
        .when(!locked, |this| {
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.set_new_connection_kind(kind, window, cx);
                    cx.stop_propagation();
                }),
            )
        })
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
        DatabaseKind::Dameng => "dameng",
        DatabaseKind::SqlServer => "sqlserver",
        DatabaseKind::Oracle => "oracle",
    }
}

fn database_kind_icon(kind: DatabaseKind) -> impl IntoElement {
    img(database_kind_icon_path(kind)).size(px(32.))
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
        DatabaseKind::Dameng => "db/dm.svg",
        DatabaseKind::SqlServer => "db/sqlserver.svg",
        DatabaseKind::Oracle => "db/oracle.svg",
    }
}

/// 新建连接表单顶部分页栏：用 gpui-component 分段 TabBar 渲染（与「连接信息」表单统一样式）。
fn connection_tab_bar(
    active_tab: NewConnectionTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let tabs = [
        (
            NewConnectionTab::Connection,
            tr("app.connection.tab.connection"),
        ),
        (NewConnectionTab::Tls, tr("app.connection.tab.tls")),
        (NewConnectionTab::Ssh, tr("app.connection.tab.ssh")),
        (
            NewConnectionTab::Advanced,
            tr("app.connection.tab.advanced"),
        ),
    ];
    let _ = colors;
    let selected_index = tabs
        .iter()
        .position(|(tab, _)| *tab == active_tab)
        .unwrap_or(0);
    let view = cx.entity();
    let tabs_for_click = tabs.clone();
    tabs.iter()
        .enumerate()
        // 页签下标是稳定业务顺序，供 UI 自动化按「连接信息/加密/SSH/高级」定位。
        .map(|(index, (_, label))| {
            Tab::from(label.clone())
                .debug_selector(move || format!("new-connection-tab-{index}"))
        })
        .fold(
            TabBar::new("new-connection-tabs")
                .segmented()
                .small()
                .selected_index(selected_index)
                .on_click(move |index, _, cx| {
                    if let Some((tab, _)) = tabs_for_click.get(*index) {
                        let _ = view.update(cx, |this, cx| {
                            this.set_new_connection_tab(*tab, cx);
                        });
                    }
                }),
            |bar, tab| bar.child(tab),
        )
        .into_element()
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
        // TLS / SSH / Advanced：Redis 与 MySQL/TiDB 都有实际表单，其余类型显示提示占位。
        NewConnectionTab::Tls => match kind {
            DatabaseKind::MySql
            | DatabaseKind::TiDb
            | DatabaseKind::MariaDb
            | DatabaseKind::Redis
            | DatabaseKind::Postgres => tls_form(kind, form, inputs, colors, window, cx),
            DatabaseKind::SqlServer => sqlserver_encryption_form(inputs, colors, window, cx),
            // Oracle 的证书校验由 Wallet（含 TCPS 凭据）承担，没有独立开关。
            DatabaseKind::Oracle => oracle_wallet_form(inputs, colors, window, cx),
            _ => redis_only_settings_hint(colors),
        },
        NewConnectionTab::Ssh => match kind {
            DatabaseKind::MySql
            | DatabaseKind::TiDb
            | DatabaseKind::MariaDb
            | DatabaseKind::Redis
            | DatabaseKind::Postgres
            | DatabaseKind::MongoDb
            | DatabaseKind::SqlServer
            | DatabaseKind::Oracle
            | DatabaseKind::Dameng => ssh_form(kind, form, inputs, colors, window, cx),
            _ => redis_only_settings_hint(colors),
        },
        NewConnectionTab::Advanced => {
            let specific = match kind {
                DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
                    mysql_advanced_form(form, inputs, colors, window, cx)
                }
                DatabaseKind::Postgres => postgres_advanced_form(form, inputs, colors, window, cx),
                DatabaseKind::Redis => advanced_form(form, inputs, colors, window, cx),
                DatabaseKind::SqlServer => sqlserver_advanced_form(inputs, colors, window, cx),
                DatabaseKind::Oracle => oracle_advanced_form(inputs, colors, window, cx),
                _ => redis_only_settings_hint(colors),
            };
            // 只读开关对所有连接类型通用，放在各库专属高级项之前。
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    h_form()
                        .label_width(px(112.))
                        .child(toggle_row_light(
                            tr("app.connection.field.read_only"),
                            ConnectionToggleField::ReadOnly,
                            form.read_only,
                            colors,
                            cx,
                        )),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(tr("app.connection.read_only.hint")),
                )
                .child(specific)
        }
    }
}

/// 非 Redis 连接在 TLS/SSH/高级页签下的提示占位。
fn redis_only_settings_hint(colors: UiColors) -> Div {
    div().flex().flex_col().gap_4().child(
        div()
            .w_full()
            .h(px(120.))
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
            .child(tr("app.connection.redis_only")),
    )
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
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label(tr("app.connection.tls.title"), colors))
        .child(h_form().label_width(px(112.)).child(toggle_row_light(
            tr("app.connection.tls.enabled"),
            ConnectionToggleField::TlsEnabled,
            form.tls_enabled,
            colors,
            cx,
        )))
        // TLS 参数明细：启用时正常显示，未启用时整体置灰。
        .when(enabled, |this| {
            this.child(tls_parameters_block(kind, form, inputs, colors, window, cx))
        })
        .when(!enabled, |this| {
            this.child(tls_parameters_block(kind, form, inputs, colors, window, cx).opacity(0.45))
        })
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
    let is_postgres = kind == DatabaseKind::Postgres;
    div()
        .flex()
        .flex_col()
        .gap_4()
        .when(is_mysql, |this| {
            this.child(mysql_tls_mode_block(form, inputs, colors, window, cx))
        })
        .when(is_postgres, |this| {
            this.child(postgres_tls_mode_block(inputs, colors, window, cx))
        })
        .child(
            h_form()
                .label_width(px(112.))
                .child(file_field_row_light(
                    tr("app.connection.tls.ca"),
                    ConnectionField::TlsCa,
                    tr("app.connection.tls.ca_prompt"),
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(file_field_row_light(
                    tr("app.connection.tls.client_cert"),
                    ConnectionField::TlsClientCert,
                    tr("app.connection.tls.client_cert_prompt"),
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(file_field_row_light(
                    tr("app.connection.tls.client_key"),
                    ConnectionField::TlsClientKey,
                    tr("app.connection.tls.client_key_prompt"),
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.tls.sni"),
                    ConnectionField::TlsSni,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(toggle_row_light(
                    tr("app.connection.tls.verify"),
                    ConnectionToggleField::TlsVerify,
                    form.tls_verify,
                    colors,
                    cx,
                )),
        )
}

/// MySQL/TiDB TLS 模式块：SSL 模式 + 连接字符集。
fn mysql_tls_mode_block(
    _form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 与「连接信息」表单一致：label 在前、输入框在后（h_form 定宽对齐）。
    div().w_full().child(
        h_form()
            .label_width(px(112.))
            .child(connection_select_field(
                tr("app.connection.tls.ssl_mode"),
                &inputs.mysql_ssl_mode_select,
                "",
            ))
            .child(field_row_light(
                tr("app.connection.tls.charset"),
                ConnectionField::MysqlCharset,
                inputs,
                false,
                colors,
                window,
                cx,
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
        h_form()
            .label_width(px(112.))
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
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label(
            tr("app.connection.sqlserver.encryption"),
            colors,
        ))
        .child(
            h_form()
                .label_width(px(112.))
                .child(connection_select_field(
                    tr("app.connection.sqlserver.encryption"),
                    &inputs.sqlserver_encryption_select,
                    "",
                ))
                .child(
                    field().label("").child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child(tr("app.connection.sqlserver.encryption_hint")),
                    ),
                ),
        )
}

/// SQL Server 高级页签：超时与应用名。
fn sqlserver_advanced_form(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div().w_full().child(
        h_form()
            .label_width(px(112.))
            .child(field_row_light(
                tr("app.connection.sqlserver.connect_timeout"),
                ConnectionField::SqlServerConnectTimeout,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                tr("app.connection.sqlserver.query_timeout"),
                ConnectionField::SqlServerQueryTimeout,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                tr("app.connection.sqlserver.application_name"),
                ConnectionField::SqlServerApplicationName,
                inputs,
                false,
                colors,
                window,
                cx,
            )),
    )
}

/// Oracle TLS 页签：Wallet 目录与口令。
/// Oracle 的证书校验通过 Wallet 承载，这里不提供「跳过校验」开关。
fn oracle_wallet_form(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label(
            tr("app.connection.oracle.wallet_title"),
            colors,
        ))
        .child(
            h_form()
                .label_width(px(112.))
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

/// Oracle 高级页签：超时与应用名。
fn oracle_advanced_form(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div().w_full().child(
        h_form()
            .label_width(px(112.))
            .child(field_row_light(
                tr("app.connection.oracle.connect_timeout"),
                ConnectionField::OracleConnectTimeout,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                tr("app.connection.oracle.query_timeout"),
                ConnectionField::OracleQueryTimeout,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                tr("app.connection.oracle.application_name"),
                ConnectionField::OracleApplicationName,
                inputs,
                false,
                colors,
                window,
                cx,
            )),
    )
}

/// PostgreSQL 高级页签：建连/查询超时、应用名、默认 schema。
fn postgres_advanced_form(
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    tr("app.connection.postgres.connect_timeout"),
                    ConnectionField::PgConnectTimeoutSecs,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.postgres.query_timeout"),
                    ConnectionField::PgQueryTimeoutSecs,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.postgres.application_name"),
                    ConnectionField::PgApplicationName,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.postgres.default_schema"),
                    ConnectionField::PgDefaultSchema,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
        .child(
            h_form()
                .label_width(px(112.))
                .child(toggle_row_light(
                    tr("app.connection.postgres.tcp_keepalive"),
                    ConnectionToggleField::PgTcpKeepalive,
                    form.pg_tcp_keepalive,
                    colors,
                    cx,
                ))
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
        )
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
    );
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label(tr("app.connection.ssh.title"), colors))
        .child(h_form().label_width(px(112.)).child(toggle_row_light(
            tr("app.connection.ssh.enabled"),
            ConnectionToggleField::SshEnabled,
            form.ssh_enabled,
            colors,
            cx,
        )))
        // 隧道参数：启用时正常显示，未启用时整体置灰。
        .when(enabled, |this| {
            this.child(ssh_tunnel_block(form, inputs, colors, window, cx))
        })
        .when(!enabled, |this| {
            this.child(ssh_tunnel_block(form, inputs, colors, window, cx).opacity(0.45))
        })
        // SSH 连接超时 + 心跳间隔（后端结构化档案已支持的类型）。
        .when(shows_tuning, |this| {
            this.child(mysql_ssh_tuning_block(inputs, colors, window, cx))
        })
        // 密码认证区块：私钥模式时置灰。
        .when(password_mode, |this| {
            this.child(ssh_password_block(inputs, colors, window, cx))
        })
        .when(!password_mode, |this| {
            this.child(ssh_password_block(inputs, colors, window, cx).opacity(0.45))
        })
        // 私钥认证区块：密码模式时置灰。
        .when(password_mode, |this| {
            this.child(ssh_private_key_block(inputs, colors, window, cx).opacity(0.45))
        })
        .when(!password_mode, |this| {
            this.child(ssh_private_key_block(inputs, colors, window, cx))
        })
}

/// SSH 隧道参数块：主机 / 端口 / 用户名 / 认证方式。
fn ssh_tunnel_block(
    _form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 与「连接信息」表单一致：label 在前、输入框在后（h_form 定宽对齐）。
    div().w_full().child(
        h_form()
            .label_width(px(112.))
            .child(field_row_light(
                tr("app.connection.field.host"),
                ConnectionField::SshHost,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                tr("app.connection.field.port"),
                ConnectionField::SshPort,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                tr("app.connection.field.username"),
                ConnectionField::SshUsername,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(connection_select_field(
                tr("app.connection.ssh.auth_method"),
                &inputs.ssh_auth_select,
                "",
            )),
    )
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
        .child(h_form().label_width(px(112.)).child(field_row_light(
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
        h_form()
            .label_width(px(112.))
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
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label(tr("app.connection.ssh.tuning"), colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    tr("app.connection.ssh.connect_timeout"),
                    ConnectionField::MysqlSshConnectTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.ssh.keepalive"),
                    ConnectionField::MysqlSshKeepalive,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
}

/// MySQL/TiDB 高级页签：代理 + 连接/查询/空闲 TTL 超时 + TCP 保活。
fn mysql_advanced_form(
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let proxy_enabled = form.mysql_proxy_enabled;
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label(
            tr("app.connection.mysql.proxy"),
            colors,
        ))
        .child(h_form().label_width(px(112.)).child(toggle_row_light(
            tr("app.connection.mysql.proxy_enabled"),
            ConnectionToggleField::MysqlProxyEnabled,
            form.mysql_proxy_enabled,
            colors,
            cx,
        )))
        // 代理参数：启用时正常显示，未启用时整体置灰。
        .when(proxy_enabled, |this| {
            this.child(mysql_proxy_block(form, inputs, colors, window, cx))
        })
        .when(!proxy_enabled, |this| {
            this.child(mysql_proxy_block(form, inputs, colors, window, cx).opacity(0.45))
        })
        .child(redis_section_label(
            tr("app.connection.mysql.advanced"),
            colors,
        ))
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    tr("app.connection.mysql.connect_timeout"),
                    ConnectionField::MysqlConnectTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.mysql.query_timeout"),
                    ConnectionField::MysqlQueryTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.mysql.idle_ttl"),
                    ConnectionField::MysqlIdleTtl,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
        .child(
            div()
                .pl_1()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(tr("app.connection.mysql.reuse_note")),
        )
        .child(h_form().label_width(px(112.)).child(toggle_row_light(
            tr("app.connection.mysql.tcp_keepalive"),
            ConnectionToggleField::MysqlTcpKeepalive,
            form.mysql_tcp_keepalive,
            colors,
            cx,
        )))
}

/// MySQL/TiDB 代理参数块：类型 / 主机 / 端口 / 用户名 / 密码。
fn mysql_proxy_block(
    _form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 与「连接信息」表单一致：label 在前、输入框在后（h_form 定宽对齐）。
    div().w_full().child(
        h_form()
            .label_width(px(112.))
            .child(field_row_light(
                tr("app.connection.mysql.proxy_type"),
                ConnectionField::MysqlProxyType,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                tr("app.connection.field.host"),
                ConnectionField::MysqlProxyHost,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                tr("app.connection.field.port"),
                ConnectionField::MysqlProxyPort,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                tr("app.connection.mysql.proxy_username"),
                ConnectionField::MysqlProxyUsername,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                tr("app.connection.mysql.proxy_password"),
                ConnectionField::MysqlProxyPassword,
                inputs,
                true,
                colors,
                window,
                cx,
            )),
    )
}

/// 高级页签：Sentinel / Cluster / 云自动发现 / 连接串导入。
fn advanced_form(
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label(
            tr("app.connection.redis.sentinel"),
            colors,
        ))
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    tr("app.connection.redis.sentinel_master"),
                    ConnectionField::SentinelMasterName,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.redis.sentinel_endpoints"),
                    ConnectionField::SentinelEndpoints,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
        .child(redis_section_label(
            tr("app.connection.redis.cluster"),
            colors,
        ))
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    tr("app.connection.redis.cluster_nodes"),
                    ConnectionField::ClusterStartNodes,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(toggle_row_light(
                    tr("app.connection.redis.cluster_readonly"),
                    ConnectionToggleField::ClusterAllowReadonly,
                    form.cluster_allow_readonly,
                    colors,
                    cx,
                )),
        )
        .child(redis_section_label(
            tr("app.connection.redis.cloud"),
            colors,
        ))
        .child(
            h_form()
                .label_width(px(112.))
                .child(connection_select_field(
                    tr("app.connection.redis.cloud_provider"),
                    &inputs.cloud_provider_select,
                    "",
                ))
                .child(field_row_light(
                    tr("app.connection.redis.cloud_subscription"),
                    ConnectionField::CloudSubscription,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    tr("app.connection.redis.cloud_resource"),
                    ConnectionField::CloudResource,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
        .child(redis_section_label(
            tr("app.connection.redis.import"),
            colors,
        ))
        .child(
            h_form()
                .label_width(px(112.))
                .child(discovery_uri_row(
                    inputs,
                    tr("app.connection.redis.uri"),
                    tr("app.connection.redis.import_action"),
                    colors,
                    window,
                    cx,
                )),
        )
}

/// 连接串导入行：URI 输入框 + 「导入/发现」按钮。
fn discovery_uri_row(
    inputs: &NewConnectionInputs,
    label: SharedString,
    action_label: SharedString,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Field {
    field()
        .label(label)
        .items_center()
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_3()
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
                        .h(px(34.))
                        .flex_none()
                        .px_3()
                        .rounded(colors.radius_lg)
                        .border_1()
                        .border_color(rgb(0x1687ff))
                        .bg(if colors.is_dark {
                            rgb(0x16324f)
                        } else {
                            rgb(0xe6f2ff)
                        })
                        .flex()
                        .items_center()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(0x1687ff))
                        .cursor_pointer()
                        .hover(move |style| style.bg(colors.hover))
                        .child(action_label)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                // 当前选中的数据库类型决定走哪套 URI 解析。
                                match this.new_connection_kind {
                                    Some(DatabaseKind::MongoDb) => {
                                        this.import_mongo_connection_string(cx)
                                    }
                                    _ => this.import_redis_connection_string(cx),
                                }
                                cx.stop_propagation();
                            }),
                        ),
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

/// 新建连接弹框里的下拉行：用 gpui-component Select 渲染，绑定根实体上的 SelectState 光标。
fn connection_select_field<I: SelectItem + 'static>(
    label: impl Into<SharedString>,
    select: &Entity<SelectState<SearchableVec<I>>>,
    search_placeholder: impl Into<SharedString>,
) -> Field {
    field().label(label.into()).items_center().child(
        Select::new(select)
            .small()
            .search_placeholder(search_placeholder),
    )
}

/// 布尔开关行：把 Checkbox 的点击写回表单对应开关字段。
fn toggle_row_light(
    label: impl Into<SharedString>,
    field_id: ConnectionToggleField,
    checked: bool,
    _colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Field {
    let view = cx.entity();
    let id = connection_toggle_id(field_id);
    let debug_id = id.to_string();
    field()
        .label(label.into())
        .items_center()
        .child(
            Checkbox::new(id)
                .debug_selector(move || debug_id.clone())
                .checked(checked)
                .on_click(move |new_checked, _, cx| {
                    let field_id = field_id;
                    let new_checked = *new_checked;
                    let _ = view.update(cx, |this, cx| {
                        this.set_connection_toggle_field(field_id, new_checked, cx);
                    });
                }),
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
    field().label(label.into()).items_center().child(
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

fn connection_form(
    kind: DatabaseKind,
    form_state: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let base = h_form()
        .label_width(px(112.))
        .child(field_row_light(
            tr("app.connection.field.name"),
            ConnectionField::Name,
            inputs,
            false,
            colors,
            window,
            cx,
        ))
        .child(color_row_light(&form_state.color, colors, cx));

    let form = match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => base
            .child(host_port_row_light(
                tr("app.connection.field.host"),
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
                tr("app.connection.field.url_params"),
                ConnectionField::UrlParams,
                inputs,
                false,
                colors,
                window,
                cx,
            )),
        DatabaseKind::Postgres => base
            .child(host_port_row_light(
                tr("app.connection.field.host"),
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
                tr("app.connection.field.url_params"),
                ConnectionField::UrlParams,
                inputs,
                false,
                colors,
                window,
                cx,
            )),
        DatabaseKind::Dameng => base
            .child(host_port_row_light(
                tr("app.connection.field.host"),
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
            ))
            .child(field_row_light(
                tr("app.connection.dameng.instance"),
                ConnectionField::Database,
                inputs,
                false,
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
            )),
        // Oracle：Service Name 与 SID 互斥，只显示当前寻址方式对应的输入框。
        DatabaseKind::Oracle => {
            let form = base
                .child(connection_select_field(
                    tr("app.connection.oracle.connect_kind"),
                    &inputs.oracle_connect_kind_select,
                    "",
                ))
                .child(host_port_row_light(
                    tr("app.connection.field.host"),
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
            let form = if form_state.oracle_connect_kind == "sid" {
                form.child(field_row_light(
                    tr("app.connection.oracle.sid"),
                    ConnectionField::OracleSid,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
            } else {
                form.child(field_row_light(
                    tr("app.connection.oracle.service_name"),
                    ConnectionField::OracleServiceName,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
            };
            form.child(field_row_light(
                tr("app.connection.oracle.schema"),
                ConnectionField::OracleSchema,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(connection_select_field(
                tr("app.connection.oracle.privilege"),
                &inputs.oracle_privilege_select,
                "",
            ))
        }
        DatabaseKind::SqlServer => base
            .child(host_port_row_light(
                tr("app.connection.field.host"),
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
                tr("app.connection.sqlserver.instance"),
                ConnectionField::SqlServerInstance,
                inputs,
                false,
                colors,
                window,
                cx,
            )),
        // 连接串导入放在最上方当主入口：Atlas 等托管服务的用户基本是粘贴 URI 起手，
        // 导入会回填下方的主机/SRV/账号/默认库/认证库，沉到表单底部等于看不见。
        DatabaseKind::MongoDb => base
            .child(discovery_uri_row(
                inputs,
                tr("app.connection.mongo.uri"),
                tr("app.connection.mongo.import_action"),
                colors,
                window,
                cx,
            ))
            .child(host_port_row_light(
                tr("app.connection.field.host"),
                inputs,
                colors,
                window,
                cx,
            ))
            // SRV 是拨号语义（DNS 拓扑发现），必须真的写进档案，不能只是装饰。
            .child(toggle_row_light(
                tr("app.connection.mongo.srv"),
                ConnectionToggleField::MongoSrv,
                form_state.mongo_srv,
                colors,
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
                tr("app.connection.field.default_db"),
                ConnectionField::MongoDefaultDb,
                inputs,
                false,
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
            )),
        DatabaseKind::Redis => base
            .child(host_port_row_light(
                tr("app.connection.field.host"),
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
            ))
            // Redis 没有连接串，TLS / Sentinel 这些开关走同一个参数输入框
            .child(field_row_light(
                tr("app.connection.field.params"),
                ConnectionField::UrlParams,
                inputs,
                false,
                colors,
                window,
                cx,
            )),
        DatabaseKind::Sqlite => base.child(sqlite_file_row_light(inputs, colors, window, cx)),
    };

    div().w_full().child(form)
}
